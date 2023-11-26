use std::{
    io::Error,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

pub enum ProxyStream {
    Http(TcpStream),
}

impl ProxyStream {
    pub fn get_stream(self) -> TcpStream {
        match self {
            ProxyStream::Http(stream) => stream,
        }
    }
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ProxyStream::Http(stream) => Pin::new(stream).poll_read(ctx, buf),
        }
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        match self.get_mut() {
            ProxyStream::Http(stream) => Pin::new(stream).poll_write(ctx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.get_mut() {
            ProxyStream::Http(stream) => Pin::new(stream).poll_flush(ctx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        match self.get_mut() {
            ProxyStream::Http(stream) => Pin::new(stream).poll_shutdown(ctx),
        }
    }
}

impl From<ProxyStream> for TcpStream {
    fn from(stream: ProxyStream) -> Self {
        match stream {
            ProxyStream::Http(stream) => stream,
        }
    }
}
