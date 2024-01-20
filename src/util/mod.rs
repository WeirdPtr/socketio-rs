#[cfg(feature = "proxy")]
pub mod base64;
#[cfg(feature = "proxy")]
pub mod proxy;

pub fn crate_user_agent() -> String {
    format!("socketio-rs v{}", env!("CARGO_PKG_VERSION"))
}

pub(crate) fn safe_spawn<F, T>(future: F)
where
    F: futures_util::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    drop(tokio::spawn(future));
}
