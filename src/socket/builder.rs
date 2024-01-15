use super::{Socket, SocketReadStream, SocketWriteSink};
use crate::{
    enums::{connection::ConnectionType, protocol::ProtocolVersion},
    get_empty_body,
    parser::Packet,
    util::crate_user_agent,
    Request,
};
use bytes::Bytes;
use fastwebsockets::handshake;
use futures_util::future::BoxFuture;
use futures_util::lock::Mutex;
use http_body_util::Empty;
use std::{collections::HashMap, sync::Arc};
use tokio::net::TcpStream;
use url::Url;

#[cfg(feature = "proxy")]
use crate::util::proxy::TcpProxy;

pub type SocketListenerFn = dyn Fn(
        Arc<Packet>,
        Arc<Mutex<SocketReadStream>>,
        Arc<Mutex<SocketWriteSink>>,
    ) -> BoxFuture<'static, ()>
    + Send
    + Sync
    + 'static;

struct SpawnExecutor;
impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
    Fut: std::future::Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    fn execute(&self, fut: Fut) {
        tokio::task::spawn(fut);
    }
}

pub struct SocketBuilder {
    protocol: ProtocolVersion,
    request: Request<Empty<Bytes>>,
    connection_type: ConnectionType,
    namespace: Option<String>,
    query: HashMap<String, String>,
    listeners: HashMap<String, Vec<Box<SocketListenerFn>>>,
    wildcard_listener: Option<Arc<Box<SocketListenerFn>>>,
    force_handshake: bool,
    #[cfg(feature = "proxy")]
    proxy: Option<String>,
    #[cfg(feature = "proxy")]
    ignore_invalid_proxy: bool,
    #[cfg(feature = "proxy")]
    ignore_proxy_env_vars: bool,
}

#[cfg(feature = "proxy")]
impl SocketBuilder {
    pub fn proxy<P>(mut self, proxy: P) -> Self
    where
        P: Into<String>,
    {
        self.proxy = Some(proxy.into());
        self
    }

    pub fn ignore_invalid_proxy(&mut self, ignore: bool) -> &mut Self {
        self.ignore_invalid_proxy = ignore;
        self
    }

    pub fn ignore_proxy_env_vars(&mut self, ignore: bool) -> &mut Self {
        self.ignore_proxy_env_vars = ignore;
        self
    }
}

impl SocketBuilder {
    pub fn new_with_request<R>(request: R) -> Self
    where
        R: Into<Request<Empty<Bytes>>> + Unpin,
    {
        let request = request.into();

        let request_uri = request.uri().to_string();

        let mut instance = SocketBuilder {
            request,
            protocol: ProtocolVersion::default(),
            connection_type: ConnectionType::default(),
            namespace: None,
            wildcard_listener: None,
            force_handshake: false,
            query: HashMap::new(),
            listeners: HashMap::new(),
            #[cfg(feature = "proxy")]
            proxy: None,
            #[cfg(feature = "proxy")]
            ignore_invalid_proxy: false,
            #[cfg(feature = "proxy")]
            ignore_proxy_env_vars: false,
        };

        instance.transform_raw_query(request_uri);

        instance
    }

    pub fn new<'a>(uri: impl Into<&'a str>) -> Self {
        let url = Url::parse(uri.into()).expect("Invalid url provided");

        let request = hyper::Request::builder()
            .uri(url.as_str())
            .header("Host", url.host_str().unwrap_or(url.as_str()))
            .header("Upgrade", "websocket")
            .header("Connection", "upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", handshake::generate_key())
            .header("User-Agent", crate_user_agent())
            .body(get_empty_body())
            .expect("Socket request builder failed");

        Self::new_with_request(request)
    }

    fn transform_raw_query<Q>(&mut self, query: Q)
    where
        Q: Into<String>,
    {
        let query: String = query.into();

        for query_entry in query.trim_start_matches('?').split('&') {
            let entry = query_entry.split_once('=');

            if entry.is_none() {
                continue;
            }

            let (mut key, value) = entry.unwrap();

            let query_params_start = key.find('?');

            if query_params_start.is_some() {
                key = key.split_at(query_params_start.unwrap()).1[1..].as_ref();
            }

            self.query.insert(key.to_owned(), value.to_owned());
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

    pub fn force_handshake(&mut self, force: bool) -> &mut Self {
        self.force_handshake = force;
        self
    }

    pub fn on<'e, E, L>(&mut self, event: E, listener: L) -> &mut Self
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
        self.listener_boxed(event, Box::new(listener));
        self
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

    #[cfg(feature = "proxy")]
    async fn establish_tunnel<'a>(&self, uri: &'a str) -> Result<TcpStream, std::io::Error> {
        let proxy_str = self.proxy.clone().unwrap_or(
            std::env::var("HTTP_PROXY")
                .unwrap_or(std::env::var("HTTPS_PROXY").unwrap_or(String::new()))
                .to_owned(),
        );

        if proxy_str.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No proxy provided",
            ));
        }

        println!("Proxy: {}", proxy_str);
        println!("URI: {}", uri);

        let stream = TcpProxy::from_proxy_str(proxy_str.as_str())?
            .connect(uri)
            .await;

        match stream {
            Ok(stream) => return Ok(stream.into()),
            Err(e) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, e)),
        }
    }

    #[inline]
    fn get_tcp_connect_str<'a, R>(request: R) -> String
    where
        R: Into<&'a Request<Empty<Bytes>>> + Unpin,
    {
        let request = request.into();
        let uri = request.uri();

        format!(
            "{}:{}",
            uri.host().expect("No host supplied"),
            uri.port_u16().unwrap_or(80)
        )
    }

    pub async fn connect(self) -> Result<Socket, Box<dyn std::error::Error>> {
        #[cfg(feature = "proxy")]
        let stream = self
            .establish_tunnel(self.request.uri().to_string().as_str())
            .await;

        #[cfg(feature = "proxy")]
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                if self.ignore_invalid_proxy {
                    TcpStream::connect(Self::get_tcp_connect_str(&self.request).as_str()).await?
                } else {
                    return Err(Box::new(e));
                }
            }
        };

        #[cfg(not(feature = "proxy"))]
        let stream = TcpStream::connect(Self::get_tcp_connect_str(&self.request).as_str()).await?;

        let ws = fastwebsockets::handshake::client(&SpawnExecutor, self.request, stream)
            .await?
            .0;

        let (read, write) = ws.split(|s| tokio::io::split(s));

        let mut socket = Socket::new(
            read,
            write,
            Some(Arc::new(Mutex::new(self.listeners))),
            self.wildcard_listener,
        );

        if self.force_handshake {
            socket.handshake().await?;
            socket.start_ping_worker().await;
        }

        Ok(socket)
    }
}
