use super::{Socket, SocketReadStream, SocketWriteSink};
use crate::{
    enums::{connection::ConnectionType, protocol::ProtocolVersion},
    parser::Packet,
};
use futures_util::lock::Mutex;
use futures_util::{future::BoxFuture, StreamExt};
use std::{collections::HashMap, sync::Arc};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::Request, protocol::WebSocketConfig,
};

pub type SocketListenerFn = dyn Fn(Packet, Arc<Mutex<SocketReadStream>>, Arc<Mutex<SocketWriteSink>>) -> BoxFuture<'static, ()>
    + Send
    + Sync
    + 'static;

pub struct SocketBuilder {
    protocol: ProtocolVersion,
    request: Request<()>,
    connection_type: ConnectionType,
    namespace: Option<String>,
    query: HashMap<String, String>,
    listeners: HashMap<String, Vec<Box<SocketListenerFn>>>,
    wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
    #[cfg(feature = "proxy")]
    proxy: Option<String>,
    ping_interval: Option<u32>,
}

impl SocketBuilder {
    pub fn new<R>(request: R) -> Self
    where
        R: IntoClientRequest + Unpin,
    {
        let request = request.into_client_request().unwrap();

        let request_uri = request.uri().to_string();

        let mut instance = SocketBuilder {
            request,
            protocol: ProtocolVersion::default(),
            connection_type: ConnectionType::default(),
            namespace: None,
            wildcard_listener: None,
            query: HashMap::new(),
            listeners: HashMap::new(),
            #[cfg(feature = "proxy")]
            proxy: None,
            ping_interval: None,
        };

        instance.transform_raw_query(request_uri);

        instance
    }

    fn transform_raw_query<Q>(&mut self, query: Q)
    where
        Q: Into<String>,
    {
        let query: String = query.into();

        for query_entry in query.trim_start_matches('?').split('&') {
            let query_entry = query_entry.split('=').collect::<Vec<&str>>();
            if query_entry.len() == 2 {
                self.query
                    .insert(query_entry[0].to_string(), query_entry[1].to_string());
            }
        }

        let engine_version = self.query.get("EIO");

        if engine_version.is_none() {
            return;
        }

        let engine_version = engine_version.unwrap().parse::<u8>();

        if engine_version.is_ok() {
            self.protocol(ProtocolVersion::from(engine_version.unwrap()));
        }
    }

    pub fn build_url(&self) -> String {
        let mut url = String::new();

        if let Some(namespace) = &self.namespace {
            if !url.ends_with('/') {
                url.push('/');
            }
            url.push_str(namespace);
        }

        if !self.query.is_empty() {
            url.push('?');
            for (key, value) in &self.query {
                url.push_str(key);
                url.push('=');
                url.push_str(value);
                url.push('&');
            }
            url.pop();
        }

        url
    }

    fn listener_boxed<'e, E>(&mut self, event: E, listener: Box<SocketListenerFn>) -> &mut Self
    where
        E: Into<&'e str>,
    {
        let event = event.into();

        if let Some(listeners) = self.listeners.get_mut(event) {
            listeners.push(listener);
        } else {
            self.listeners.insert(event.to_owned(), vec![listener]);
        }
        self
    }

    pub fn protocol(&mut self, protocol: ProtocolVersion) -> &mut Self {
        self.protocol = protocol;
        self
    }

    pub fn connection_type(&mut self, connection_type: ConnectionType) -> &mut Self {
        self.connection_type = connection_type;
        self
    }

    pub fn namespace<N>(&mut self, namespace: N) -> &mut Self
    where
        N: Into<String>,
    {
        self.namespace = Some(namespace.into());
        self
    }

    pub fn query<Q>(&mut self, query: Q) -> &mut Self
    where
        Q: Into<HashMap<String, String>>,
    {
        self.query = query.into();
        self
    }

    #[cfg(feature = "proxy")]
    pub fn proxy<P>(mut self, proxy: P) -> Self
    where
        P: Into<String>,
    {
        self.proxy = Some(proxy.into());
        self
    }

    pub fn listeners(
        &mut self,
        listeners: HashMap<String, Vec<Box<SocketListenerFn>>>,
    ) -> &mut Self {
        self.listeners = listeners;
        self
    }

    pub fn raw_query<Q>(&mut self, query: Q) -> &mut Self
    where
        Q: Into<String>,
    {
        self.query.clear();
        self.transform_raw_query(query);
        self
    }

    pub fn on<'e, E, L>(mut self, event: E, listener: L) -> Self
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
        self.listener_boxed(event, Box::new(listener));
        self
    }

    pub fn on_any<L>(mut self, listener: L) -> Self
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

    pub fn ping_interval(&mut self, ping_interval: u32) -> &mut Self {
        self.ping_interval = Some(ping_interval);
        self
    }

    #[cfg(not(feature = "proxy"))]
    pub async fn connect(self) -> Result<Socket, Box<dyn std::error::Error>> {
        use tokio_tungstenite::connect_async_with_config;

        let config = WebSocketConfig {
            max_message_size: Some(usize::MAX),
            max_frame_size: Some(usize::MAX),
            accept_unmasked_frames: true,
            ..Default::default()
        };

        let (ws_stream, _) =
            connect_async_with_config(self.request.uri().to_owned(), Some(config), false).await?;

        let (write, read) = ws_stream.split();

        let socket = Socket::new(
            read,
            write,
            Some(Arc::new(Mutex::new(self.listeners))),
            self.wildcard_listener,
            self.ping_interval,
        );

        Ok(socket)
    }

    #[cfg(feature = "proxy")]
    pub async fn connect(self) -> Result<Socket, Box<dyn std::error::Error>> {
        use crate::util::proxy::inner::InnerProxy;
        use tokio::net::TcpStream;
        use tokio_tungstenite::client_async_with_config;

        let config = WebSocketConfig {
            max_message_size: Some(usize::MAX),
            max_frame_size: Some(usize::MAX),
            accept_unmasked_frames: true,
            ..Default::default()
        };

        let proxy = self.proxy.clone().unwrap_or(
            std::env::var("HTTP_PROXY")
                .unwrap_or(std::env::var("HTTPS_PROXY").unwrap_or(String::new()))
                .to_owned(),
        );

        if proxy.is_empty() {
            return Err("No proxy provided".into());
        }

        let proxy = InnerProxy::from_proxy_str(proxy.as_str())?;

        let proxy_stream: TcpStream = proxy
            .connect_async(self.request.uri().to_string().as_str())
            .await?
            .into();

        let (write, read) = client_async_with_config(self.request, proxy_stream, Some(config))
            .await?
            .0
            .split();

        let socket = Socket::new(
            read,
            write,
            Some(Arc::new(Mutex::new(self.listeners))),
            self.wildcard_listener,
            self.ping_interval,
        );

        socket.handshake().await?;

        Ok(socket)
    }
}
