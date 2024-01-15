use crate::util::base64::base64_encode;
use std::io::{Error, ErrorKind};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::{Position, Url};

use self::stream::ProxyStream;

pub mod stream;

#[derive(Debug)]
pub enum TcpProxy {
    Http { auth: Option<Vec<u8>>, url: String },
}

impl TcpProxy {
    pub fn from_proxy_str(proxy_str: &str) -> Result<TcpProxy, Error> {
        let url = match Url::parse(proxy_str) {
            Ok(u) => u,
            Err(_) => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "Invalid proxy url provided",
                ))
            }
        };
        let addr = &url[Position::BeforeHost..Position::AfterPort];

        // returns scheme as lowercase
        match url.scheme() {
            "http" | "https" => {
                let mut basic_auth_bytes: Option<Vec<u8>> = None;
                if let Some(password) = url.password() {
                    let encoded_str = format!(
                        "Basic {}",
                        base64_encode(&format!("{username}:{password}", username = url.username()))
                    );
                    basic_auth_bytes = Some(encoded_str.into_bytes());
                };
                Ok(TcpProxy::Http {
                    auth: basic_auth_bytes,
                    url: addr.to_string(),
                })
            }
            _ => Err(Error::new(ErrorKind::Unsupported, "Unknown schema")),
        }
    }

    pub async fn connect(&self, target: &str) -> Result<ProxyStream, Error> {
        let target_url =
            Url::parse(target).map_err(|_| Error::new(ErrorKind::InvalidInput, "Invalid uri"))?;

        let host = match target_url.host_str() {
            Some(host) => host.to_string(),
            None => return Err(Error::new(ErrorKind::Unsupported, "Target not reachable")),
        };

        let port = target_url.port().unwrap_or(80);

        match self {
            TcpProxy::Http { auth, url } => {
                let tcp_stream = TcpStream::connect(url)
                    .await
                    .expect("Failed to connect to proxy");
                Ok(ProxyStream::Http(
                    Self::tunnel(tcp_stream, host, port, auth).await.unwrap(),
                ))
            }
        }
    }

    async fn tunnel(
        mut conn: TcpStream,
        host: String,
        port: u16,
        auth: &Option<Vec<u8>>,
    ) -> Result<TcpStream, Error> {
        let mut buf = format!(
            "\
         CONNECT {host}:{port} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         ",
        )
        .into_bytes();

        if let Some(auth) = auth {
            buf.extend_from_slice(b"Proxy-Authorization: ");
            buf.extend_from_slice(auth.as_slice());
            buf.extend_from_slice(b"\r\n");
        }

        buf.extend_from_slice(b"\r\n");
        conn.write_all(&buf).await?;

        let mut buf = [0; 1024];
        let mut buf_pos = 0;

        loop {
            let response_bytes = conn.read(&mut buf[buf_pos..]).await?;
            if response_bytes == 0 {
                return Err(Error::new(ErrorKind::UnexpectedEof, "Response empty"));
            }

            buf_pos += response_bytes;

            let recvd = &buf[..buf_pos];

            if recvd.starts_with(b"HTTP/1.1 200") || recvd.starts_with(b"HTTP/1.0 200") {
                if recvd.ends_with(b"\r\n\r\n") {
                    return Ok(conn);
                }
                if buf_pos == buf.len() {
                    return Err(Error::new(ErrorKind::UnexpectedEof, "Headers too long"));
                }
            } else if recvd.starts_with(b"HTTP/1.1 407") {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "Proxy authentication required",
                ));
            } else {
                return Err(Error::new(ErrorKind::Other, "Proxy error"));
            }
        }
    }
}
