#[cfg(feature = "proxy")]
pub mod base64;
#[cfg(feature = "proxy")]
pub mod proxy;

pub fn crate_user_agent() -> String {
    format!("socketio-rs v{}", env!("CARGO_PKG_VERSION"))
}
