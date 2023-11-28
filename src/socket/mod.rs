use self::builder::SocketListenerFn;
use crate::{enums::packet::PacketType, parser::Packet, structs::handshake::Handshake};
use futures_util::{
    future::BoxFuture,
    lock::Mutex,
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use std::{collections::HashMap, sync::Arc, thread};
use tokio::{join, net::TcpStream};
#[cfg(not(feature = "proxy"))]
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub mod builder;

#[cfg(not(feature = "proxy"))]
pub type SocketWriteSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
#[cfg(not(feature = "proxy"))]
pub type SocketReadStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

#[cfg(feature = "proxy")]
pub type SocketWriteSink = SplitSink<WebSocketStream<TcpStream>, Message>;
#[cfg(feature = "proxy")]
pub type SocketReadStream = SplitStream<WebSocketStream<TcpStream>>;

pub struct Socket {
    read: Arc<Mutex<SocketReadStream>>,
    write: Arc<Mutex<SocketWriteSink>>,
    listeners: Arc<Mutex<HashMap<String, Vec<Box<SocketListenerFn>>>>>,
    wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
    /// Ping interval in ms
    ping_interval: u32,
}

impl Socket {
    pub fn new(
        read: SocketReadStream,
        write: SocketWriteSink,
        listeners: Option<Arc<Mutex<HashMap<String, Vec<Box<SocketListenerFn>>>>>>,
        wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
        ping_interval: Option<u32>,
    ) -> Self {
        Self {
            read: Arc::new(Mutex::new(read)),
            write: Arc::new(Mutex::new(write)),
            listeners: listeners.unwrap_or(Arc::new(Mutex::new(HashMap::new()))),
            wildcard_listener,
            ping_interval: ping_interval.unwrap_or(25_000),
        }
    }

    pub async fn run(
        &mut self,
    ) -> (
        Result<(), tokio::task::JoinError>,
        Result<(), tokio::task::JoinError>,
    ) {
        let ping_write = self.write.clone();

        let ping_listeners = self.listeners.clone();

        let ping_interval = self.ping_interval;

        let ping_read_guard = self.read();
        let ping_write_guard = self.write();

        let ping_handle = tokio::spawn(async move {
            let ping_interval =
                std::time::Duration::from_millis(ping_interval.try_into().unwrap_or(25_000));

            loop {
                thread::sleep(ping_interval);

                let listeners = ping_listeners.lock().await;

                listeners
                    .get("ping")
                    .unwrap_or(&vec![])
                    .iter()
                    .for_each(|listener| {
                        tokio::spawn(listener(
                            Packet::new(PacketType::Ping, None, None, None),
                            ping_read_guard.clone(),
                            ping_write_guard.clone(),
                        ));
                    });

                let ping_result = Self::inner_ping(ping_write.clone()).await;

                if ping_result.is_err() {
                    continue;
                }

                listeners
                    .get("pong")
                    .unwrap_or(&vec![])
                    .iter()
                    .for_each(|listener| {
                        tokio::spawn(listener(
                            Packet::new(PacketType::Ping, None, None, None),
                            ping_read_guard.clone(),
                            ping_write_guard.clone(),
                        ));
                    });
            }
        });

        let worker_read = self.read();

        let wildcard_listener = self
            .wildcard_listener
            .as_ref()
            .unwrap_or(&Arc::new(Box::new(|_, _, _| Box::pin(async {}))))
            .clone();

        let listener_guard = self.listeners.clone();

        let worker_read_guard = self.read();
        let worker_write_guard = self.write();

        let worker_handle = tokio::spawn(async move {
            loop {
                let msg = worker_read.lock().await.next().await;

                if msg.is_none() {
                    continue;
                }

                let msg = msg.unwrap();

                if msg.is_err() {
                    continue;
                }

                let msg = msg.unwrap();

                match msg {
                    Message::Text(text) => {
                        let listener_guard = listener_guard.lock().await;

                        let packet = Packet::decode(text);

                        if packet.is_err() {
                            continue;
                        }

                        let packet = packet.unwrap();

                        if let Some(target) = &packet.target {
                            if let Some(listeners) = listener_guard.get(target) {
                                listeners.iter().for_each(|listener| {
                                    // TODO: get rid of clone
                                    tokio::spawn(listener(
                                        packet.clone(),
                                        worker_read_guard.clone(),
                                        worker_write_guard.clone(),
                                    ));
                                });
                            }
                        }

                        tokio::spawn(wildcard_listener(
                            packet,
                            worker_read_guard.clone(),
                            worker_write_guard.clone(),
                        ));
                    }
                    Message::Close(_) => listener_guard
                        .lock()
                        .await
                        .get("close")
                        .unwrap_or(&vec![])
                        .iter()
                        .for_each(|listener| {
                            tokio::spawn(listener(
                                Packet::new(PacketType::Event, None, None, None),
                                worker_read_guard.clone(),
                                worker_write_guard.clone(),
                            ));
                        }),
                    _ => { /* Ignore */ }
                }
            }
        });

        join!(ping_handle, worker_handle)
    }

    pub fn run_background(mut self) {
        tokio::spawn(async move {
            let _ = self.run().await;
        });
    }

    async fn listener_boxed<'e, E>(
        &mut self,
        event: E,
        listener: Box<SocketListenerFn>,
    ) -> &mut Self
    where
        E: Into<&'e str>,
    {
        let event = event.into();

        if event == "*" {
            self.wildcard_listener = Some(Arc::new(listener));
        } else {
            let mut listener_guard = self.listeners.lock().await;

            if listener_guard.contains_key(event) {
                listener_guard.get_mut(event).unwrap().push(listener);
            } else {
                listener_guard.insert(event.to_owned(), vec![listener]);
            }
        }

        self
    }

    async fn inner_ping(
        write: Arc<Mutex<SocketWriteSink>>,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        Self::send_raw(write, PacketType::Ping.into()).await
    }

    pub async fn on<'e, E, L>(&mut self, event: E, listener: L) -> &mut Self
    where
        E: Into<&'e str>,
        L: for<'a> Fn(
                Packet,
                Arc<Mutex<SocketReadStream>>,
                Arc<Mutex<SocketWriteSink>>,
            ) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
    {
        self.listener_boxed(event, Box::new(listener)).await;
        self
    }

    pub async fn emit(&self, event: String, data: Packet) {
        let listeners = self.listeners.lock().await;
        let listeners = listeners.get(&event);

        if listeners.is_some() {
            let listeners = listeners.unwrap();

            listeners.iter().for_each(|listener| {
                tokio::spawn(listener(data.clone(), self.read(), self.write()));
            });
        }

        if self.wildcard_listener.is_some() {
            let listener =
                self.wildcard_listener.as_ref().unwrap()(data, self.read(), self.write());

            tokio::spawn(listener);
        }
    }

    pub fn on_any<L>(&mut self, listener: L) -> &mut Self
    where
        L: for<'a> Fn(
                Packet,
                Arc<Mutex<SocketReadStream>>,
                Arc<Mutex<SocketWriteSink>>,
            ) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
    {
        self.wildcard_listener = Some(Arc::new(Box::new(listener)));
        self
    }

    pub fn read(&self) -> Arc<Mutex<SocketReadStream>> {
        self.read.clone()
    }

    pub fn write(&self) -> Arc<Mutex<SocketWriteSink>> {
        self.write.clone()
    }

    pub async fn ping(&self) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        Self::inner_ping(self.write.clone()).await
    }

    pub async fn handshake(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let handshake = Handshake {
            sid: None,
            upgrades: vec!["websocket".to_string()],
            ping_timeout: 20000,
            ping_interval: self.ping_interval,
        };

        let handshake_packet = Packet::new_raw(
            PacketType::Connect,
            None,
            None,
            serde_json::to_string(&handshake)?,
        );

        Self::send_raw_packet(self.write(), handshake_packet).await?;

        let handshake_response = self.read.lock().await.next().await;

        if handshake_response.is_some() {
            let response = handshake_response.unwrap().unwrap().to_string();

            let packet = Packet::decode(response.as_bytes().to_vec())?;

            self.emit("handshake".to_owned(), packet).await;
        }

        Ok(())
    }

    pub async fn send(&self, data: impl AsRef<[u8]>) -> Result<(), Box<dyn std::error::Error>> {
        let packet = Packet::new_raw(
            PacketType::Event,
            None,
            None,
            String::from_utf8(data.as_ref().to_vec())?,
        );

        self.send_packet(packet).await?;

        Ok(())
    }

    pub async fn send_raw_with_type(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: Message,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        write.lock().await.send(payload).await.map_err(|e| e.into())
    }

    pub async fn send_raw(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: String,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        Self::send_raw_with_type(write, Message::Text(payload)).await
    }

    pub async fn send_raw_packet(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: Packet,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        Self::send_raw(write, Packet::encode(payload)).await
    }

    pub async fn send_packet(&self, packet: Packet) -> Result<(), Box<dyn std::error::Error>> {
        Self::send_raw_packet(self.write(), packet)
            .await
            .map_err(|e| e.into())
    }
}
