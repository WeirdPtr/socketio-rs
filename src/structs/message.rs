use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SocketIOMessage {
    #[serde(rename = "type")]
    pub msg_type: i32,
    pub nsp: String,
    pub data: String,
}
