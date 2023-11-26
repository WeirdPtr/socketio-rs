/// The type of connection to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConnectionType {
    WebSocket,
}

impl Default for ConnectionType {
    fn default() -> Self {
        ConnectionType::WebSocket
    }
}

impl ToString for ConnectionType {
    fn to_string(&self) -> String {
        match self {
            ConnectionType::WebSocket => "websocket".to_string(),
        }
    }
}

impl From<ConnectionType> for String {
    fn from(value: ConnectionType) -> Self {
        value.to_string()
    }
}
