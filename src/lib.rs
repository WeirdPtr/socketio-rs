pub mod enums;
pub mod parser;
pub mod socket;
pub mod structs;
pub mod util;

/// Re-export of `tokio_tungstenite::tungstenite::http::Request`
pub type Request<T> = tokio_tungstenite::tungstenite::http::Request<T>;

/// Re-export of `tokio_tungstenite::tungstenite::Message`
pub type Message = tokio_tungstenite::tungstenite::Message;
