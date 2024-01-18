#[derive(Clone)]
pub struct ReconnectConfiguration {
    pub enable_reconnect: bool,
    pub reconnect_count: Option<u64>,
    pub reconnect_delay: u64,
    pub request: hyper::Request<http_body_util::Empty<bytes::Bytes>>,
    pub force_handshake: bool,
    #[cfg(feature = "proxy")]
    pub ignore_invalid_proxy: bool,
    #[cfg(feature = "proxy")]
    pub ignore_proxy_env_vars: bool,
    #[cfg(feature = "proxy")]
    pub proxy: Option<String>,
}
