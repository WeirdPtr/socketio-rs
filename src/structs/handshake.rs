use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Handshake {
    pub sid: Option<String>,
    pub upgrades: Vec<String>,
    #[serde(rename = "pingTimeout")]
    pub ping_timeout: u32,
    #[serde(rename = "pingInterval")]
    pub ping_interval: u32,
}
