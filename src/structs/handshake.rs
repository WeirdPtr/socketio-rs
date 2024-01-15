use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handshake {
    pub sid: Option<String>,
    pub upgrades: Vec<String>,
    #[serde(rename = "pingTimeout")]
    pub ping_timeout: u64,
    #[serde(rename = "pingInterval")]
    pub ping_interval: u64,
}
