pub mod enums;
pub mod parser;
pub mod socket;
pub mod structs;
pub mod util;

/// Re-export of `hyper::Request`
pub type Request<T> = hyper::Request<T>;

/// Re-export
pub fn get_empty_body() -> http_body_util::Empty<bytes::Bytes> {
    http_body_util::Empty::new()
}
