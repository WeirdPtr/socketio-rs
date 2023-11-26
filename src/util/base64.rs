pub fn base64_encode(string: impl AsRef<[u8]>) -> String {
    base64::Engine::encode(&base64::prelude::BASE64_STANDARD, string)
}