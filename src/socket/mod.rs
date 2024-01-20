use self::builder::{SocketBuilder, SocketListenerFn};
use crate::{
    enums::packet::PacketType,
    parser::Packet,
    structs::{handshake::Handshake, reconnect::ReconnectConfiguration},
    util::safe_spawn,
};
use fastwebsockets::{Frame, OpCode, WebSocketError, WebSocketRead, WebSocketWrite};
use futures_util::{
    future::{abortable, BoxFuture},
    lock::Mutex,
    FutureExt,
};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use std::{collections::HashMap, sync::Arc, thread};
use tokio::io::{ReadHalf, WriteHalf};

pub mod builder;

pub type SocketReadStream = WebSocketRead<ReadHalf<TokioIo<Upgraded>>>;
pub type SocketWriteSink = WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>;

/// Re-export of `fastwebsockets::Payload`
pub type SocketPayload<'a> = fastwebsockets::Payload<'a>;

pub struct Socket {
    read: Arc<Mutex<SocketReadStream>>,
    write: Arc<Mutex<SocketWriteSink>>,
    listeners: Arc<Mutex<HashMap<String, Vec<Box<SocketListenerFn>>>>>,
    wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
    handshake_response: Option<Handshake>,
    worker_handles: Option<(
        tokio::task::JoinHandle<Result<(), futures_util::stream::Aborted>>,
        futures_util::stream::AbortHandle,
    )>,
    ping_worker_handles: Option<(
        tokio::task::JoinHandle<Result<(), futures_util::stream::Aborted>>,
        futures_util::stream::AbortHandle,
    )>,
    reconnect_configuration: Option<ReconnectConfiguration>,
}

impl Socket {
    pub fn new(
        read: impl Into<Arc<Mutex<SocketReadStream>>>,
        write: impl Into<Arc<Mutex<SocketWriteSink>>>,
        listeners: Option<Arc<Mutex<HashMap<String, Vec<Box<SocketListenerFn>>>>>>,
        wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
    ) -> Self {
        Self {
            read: read.into(),
            write: write.into(),
            listeners: listeners.unwrap_or(Arc::new(Mutex::new(HashMap::new()))),
            wildcard_listener,
            handshake_response: None,
            worker_handles: None,
            ping_worker_handles: None,
            reconnect_configuration: None,
        }
    }

    pub async fn run(&mut self) {
        // let _ = self.stop_workers().await;

        let worker_read = self.read();
        let wildcard_listener = self.wildcard_listener.clone();
        let listener_guard = self.listeners.clone();
        let worker_read_guard = self.read();
        let worker_write_guard = self.write();

        let (task, handle) = abortable(async move {
            loop {
                let mut frame = worker_read.lock().await;

                let frame = frame
                    .read_frame::<_, WebSocketError>(&mut |frame| {
                        let listener_guard = listener_guard.clone();
                        let wildcard_listener = wildcard_listener.clone();
                        let worker_read_guard = worker_read_guard.clone();
                        let worker_write_guard = worker_write_guard.clone();

                        async move {
                            match frame.opcode {
                                OpCode::Close => {
                                    Self::emit_raw(
                                        "close",
                                        listener_guard.clone(),
                                        wildcard_listener.clone(),
                                        Packet::new(PacketType::Event, None, None, None),
                                        worker_read_guard.clone(),
                                        worker_write_guard.clone(),
                                    )
                                    .await;
                                    Ok(())
                                }
                                _ => Err(WebSocketError::IoError(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    "Listener failed",
                                ))),
                            }
                        }
                    })
                    .await;

                let frame = match frame {
                    Ok(frame) => frame,
                    Err(e) => match e {
                        WebSocketError::IoError(e) => {
                            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                                Self::emit_raw(
                                    "close",
                                    listener_guard.clone(),
                                    wildcard_listener.clone(),
                                    Packet::new(
                                        PacketType::Event,
                                        None,
                                        None,
                                        Some(serde_json::Value::String(
                                            "Unexpected EOF".to_owned(),
                                        )),
                                    ),
                                    worker_read_guard.clone(),
                                    worker_write_guard.clone(),
                                )
                                .await;
                            }
                            break;
                        }
                        WebSocketError::UnexpectedEOF => {
                            Self::emit_raw(
                                "close",
                                listener_guard.clone(),
                                wildcard_listener.clone(),
                                Packet::new(
                                    PacketType::Event,
                                    None,
                                    None,
                                    Some(serde_json::Value::String("Unexpected EOF".to_owned())),
                                ),
                                worker_read_guard.clone(),
                                worker_write_guard.clone(),
                            )
                            .await;
                            break;
                        }
                        err => {
                            Self::emit_raw(
                                "close",
                                listener_guard.clone(),
                                wildcard_listener.clone(),
                                Packet::new(
                                    PacketType::Event,
                                    None,
                                    None,
                                    Some(serde_json::Value::String(format!("Unknown: {:?}", err))),
                                ),
                                worker_read_guard.clone(),
                                worker_write_guard.clone(),
                            )
                            .await;
                            break;
                        }
                    },
                };

                match frame.opcode {
                    OpCode::Text | OpCode::Binary => {
                        let text = String::from_utf8(frame.payload.to_vec());

                        let text = match text {
                            Ok(text) => text,
                            Err(_) => continue,
                        };

                        let listener_guard = listener_guard.lock().await;

                        let packet = Packet::decode(text);

                        if packet.is_err() {
                            continue;
                        }

                        let packet = Arc::new(packet.unwrap());

                        if packet.packet_type == PacketType::Connect {
                            if let Some(listeners) = listener_guard.get("handshake") {
                                listeners.iter().for_each(|listener| {
                                    safe_spawn(listener(
                                        packet.clone(),
                                        worker_read_guard.clone(),
                                        worker_write_guard.clone(),
                                    ));
                                });
                            }
                        }

                        if let Some(target) = &packet.target {
                            if let Some(listeners) = listener_guard.get(target) {
                                listeners.iter().for_each(|listener| {
                                    safe_spawn(listener(
                                        packet.clone(),
                                        worker_read_guard.clone(),
                                        worker_write_guard.clone(),
                                    ));
                                });
                            }
                        }

                        if let Some(wildcard_listener) = wildcard_listener.clone() {
                            safe_spawn(wildcard_listener(
                                packet.clone(),
                                worker_read_guard.clone(),
                                worker_write_guard.clone(),
                            ));
                        }
                    }
                    OpCode::Close => {
                        safe_spawn(Self::emit_raw(
                            "close",
                            listener_guard.clone(),
                            wildcard_listener.clone(),
                            Packet::new(
                                PacketType::Event,
                                None,
                                None,
                                Some(serde_json::Value::String("Close".to_owned())),
                            ),
                            worker_read_guard.clone(),
                            worker_write_guard.clone(),
                        ));
                        break;
                    }
                    _ => { /* Ignore */ }
                }
            }
        });

        let worker_handle = tokio::spawn(task);

        self.worker_handles = Some((worker_handle, handle));
    }

    async fn start_ping_worker(&mut self) {
        let ping_write = self.write.clone();

        let ping_read_guard = self.read();
        let ping_write_guard = self.write();
        let ping_listeners = self.listeners.clone();
        let ping_wildcard_listener = self.wildcard_listener.clone();

        let ping_interval = match self.handshake_response.as_ref() {
            Some(handshake_response) => handshake_response.ping_interval,
            None => 25_000,
        };

        let (task, handle) = abortable(async move {
            let ping_interval = std::time::Duration::from_millis(ping_interval);

            loop {
                safe_spawn(Self::emit_raw(
                    "ping",
                    ping_listeners.clone(),
                    ping_wildcard_listener.clone(),
                    Packet::new(PacketType::Ping, None, None, None),
                    ping_read_guard.clone(),
                    ping_write_guard.clone(),
                ));

                let ping_result = Self::inner_ping(ping_write.clone()).await;

                if ping_result.is_err() {
                    continue;
                }

                safe_spawn(Self::emit_raw(
                    "pong",
                    ping_listeners.clone(),
                    ping_wildcard_listener.clone(),
                    Packet::new(PacketType::Ping, None, None, None),
                    ping_read_guard.clone(),
                    ping_write_guard.clone(),
                ));

                thread::sleep(ping_interval);
            }
        });

        let ping_handle = tokio::spawn(task);

        self.ping_worker_handles = Some((ping_handle, handle));
    }

    pub fn run_background(mut self) {
        safe_spawn(async move {
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

    async fn inner_ping(write: Arc<Mutex<SocketWriteSink>>) -> Result<(), WebSocketError> {
        let ping_payload: &str = PacketType::Ping.into();
        Self::send_raw(write, ping_payload.as_bytes()).await
    }

    pub async fn on<'e, E, L>(&mut self, event: E, listener: L) -> &mut Self
    where
        E: Into<&'e str>,
        L: for<'a> Fn(
                Arc<Packet>,
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

        let packet = Arc::new(data);

        if listeners.is_some() {
            let listeners = listeners.unwrap();

            listeners.iter().for_each(|listener| {
                safe_spawn(listener(packet.clone(), self.read(), self.write()));
            });
        }

        if self.wildcard_listener.is_some() {
            safe_spawn(self.wildcard_listener.as_ref().unwrap()(
                packet,
                self.read(),
                self.write(),
            ));
        }
    }

    pub fn on_any<L>(&mut self, listener: L) -> &mut Self
    where
        L: for<'a> Fn(
                Arc<Packet>,
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

    pub async fn ping(&self) -> Result<(), WebSocketError> {
        Self::inner_ping(self.write.clone()).await
    }

    pub async fn handshake(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let handshake = Handshake {
            sid: None,
            upgrades: vec!["websocket".to_string()],
            ping_timeout: 20000,
            ping_interval: 25000,
        };

        let handshake_packet = Packet::new_raw(
            PacketType::Connect,
            None,
            None,
            serde_json::to_string(&handshake)?,
        );

        Self::send_raw_packet(self.write(), handshake_packet).await?;

        let mut handshake_response_frame = self.read.lock().await;

        let handshake_response_frame = handshake_response_frame
            .read_frame::<_, WebSocketError>(&mut |_| {
                async {
                    Err(WebSocketError::IoError(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Handshake failed",
                    )))
                }
                .boxed()
            })
            .await?;

        let handshake_response_packet = match handshake_response_frame.opcode {
            OpCode::Text | OpCode::Binary => {
                Packet::decode(handshake_response_frame.payload.to_vec()).ok()
            }
            _ => None,
        };

        if handshake_response_packet.is_none() {
            return Ok(());
        }

        let handshake_response_packet = handshake_response_packet.unwrap();

        if let Some(handshake_data) = handshake_response_packet.data.clone() {
            let handshake_data: Handshake = serde_json::from_value(handshake_data)?;

            self.handshake_response = Some(handshake_data);
        }

        self.emit("handshake".to_owned(), handshake_response_packet)
            .await;

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

    pub async fn send_raw_payload(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: impl Into<SocketPayload<'_>>,
    ) -> Result<(), WebSocketError> {
        write
            .lock()
            .await
            .write_frame(Frame::text(payload.into()))
            .await
    }

    pub async fn send_raw<'p>(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: impl Into<&'p [u8]>,
    ) -> Result<(), WebSocketError> {
        Self::send_raw_payload(write, SocketPayload::Borrowed(payload.into())).await
    }

    pub async fn send_raw_packet(
        write: Arc<Mutex<SocketWriteSink>>,
        payload: Packet,
    ) -> Result<(), WebSocketError> {
        Self::send_raw(write, Packet::encode(payload).as_bytes()).await
    }

    pub async fn send_packet(&self, packet: Packet) -> Result<(), WebSocketError> {
        Self::send_raw_packet(self.write(), packet).await
    }

    pub async fn emit_raw<'a>(
        event_identifier: impl Into<&'a str>,
        listeners: Arc<Mutex<HashMap<String, Vec<Box<SocketListenerFn>>>>>,
        wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
        packet: impl Into<Arc<Packet>>,
        read: Arc<Mutex<WebSocketRead<ReadHalf<TokioIo<Upgraded>>>>>,
        write: Arc<Mutex<WebSocketWrite<WriteHalf<TokioIo<Upgraded>>>>>,
    ) {
        let packet = packet.into();

        listeners
            .lock()
            .await
            .get(event_identifier.into())
            .unwrap_or(&vec![])
            .iter()
            .for_each(|listener| {
                safe_spawn(listener(packet.clone(), read.clone(), write.clone()));
            });

        if let Some(wildcard_listener) = wildcard_listener {
            safe_spawn(wildcard_listener(packet, read, write));
        }
    }

    pub async fn stop_ping_worker(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some((_, abort_handle)) = self.ping_worker_handles.take() {
            abort_handle.abort();
            safe_spawn(Self::emit_raw(
                "worker:stopped",
                self.listeners.clone(),
                self.wildcard_listener.clone(),
                Packet::new(
                    PacketType::Event,
                    None,
                    None,
                    Some(serde_json::Value::String("ping".to_owned())),
                ),
                self.read(),
                self.write(),
            ));
        }

        Ok(())
    }

    pub async fn stop_listener_worker(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some((_, abort_handle)) = self.worker_handles.take() {
            abort_handle.abort();
            safe_spawn(Self::emit_raw(
                "worker:stopped",
                self.listeners.clone(),
                self.wildcard_listener.clone(),
                Packet::new(
                    PacketType::Event,
                    None,
                    None,
                    Some(serde_json::Value::String("listener".to_owned())),
                ),
                self.read(),
                self.write(),
            ));
        }

        Ok(())
    }

    pub async fn stop_workers(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.stop_ping_worker().await?;
        self.stop_listener_worker().await?;
        Ok(())
    }

    pub fn reconnect_configuration(&mut self, configuration: ReconnectConfiguration) -> &mut Self {
        self.reconnect_configuration = Some(configuration);
        self
    }

    pub fn get_reconnect_configuration(&self) -> Option<&ReconnectConfiguration> {
        self.reconnect_configuration.as_ref()
    }

    pub async fn reconnect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.reconnect_configuration.is_none() {
            return Err("Reconnect configuration is not set".into());
        }

        let configuration = self.reconnect_configuration.clone().unwrap();

        if !configuration.enable_reconnect {
            return Err("Reconnect is disabled".into());
        }

        self.stop_listener_worker().await?;

        Self::reconnect_raw(self.read.clone(), self.write.clone(), configuration.clone()).await?;

        self.emit(
            "open".to_owned(),
            Packet::new(
                PacketType::Event,
                None,
                Some("open".to_owned()),
                Some(serde_json::Value::String("open".to_owned())),
            ),
        )
        .await;

        if configuration.force_handshake {
            self.stop_ping_worker().await?;
            self.handshake().await?;
            self.start_ping_worker().await;
        }

        self.run().await;

        Self::emit_raw(
            "reconnect",
            self.listeners.clone(),
            self.wildcard_listener.clone(),
            Packet::new(PacketType::Event, None, None, None),
            self.read(),
            self.write(),
        )
        .await;

        Self::emit_raw(
            "reconnect",
            self.listeners.clone(),
            self.wildcard_listener.clone(),
            Packet::new(PacketType::Event, None, None, None),
            self.read(),
            self.write(),
        )
        .await;

        Ok(())
    }

    /// Reconnects the socket with the given configuration <br>
    /// <b>DOES NOT EMIT THE `reconnect` EVENT</b>
    pub async fn reconnect_raw(
        read: Arc<Mutex<SocketReadStream>>,
        write: Arc<Mutex<SocketWriteSink>>,
        configuration: ReconnectConfiguration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !configuration.enable_reconnect {
            return Err("Reconnect is disabled".into());
        }

        let mut read_guard = read.lock().await;
        let mut write_guard = write.lock().await;

        let mut reconnect_count: u64 = 0;

        loop {
            if configuration.reconnect_count.is_some() {
                if reconnect_count >= configuration.reconnect_count.unwrap() {
                    break;
                }

                if reconnect_count > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        configuration.reconnect_delay,
                    ))
                    .await;
                }
            }

            reconnect_count += 1;

            let response = SocketBuilder::connect_with_reconnect_config(&configuration).await;

            if response.is_err() {
                continue;
            }

            let (read, write) = response.unwrap();

            drop(std::mem::replace(&mut *read_guard, read));
            drop(std::mem::replace(&mut *write_guard, write));

            break;
        }

        drop(read_guard);
        drop(write_guard);

        Ok(())
    }
}
