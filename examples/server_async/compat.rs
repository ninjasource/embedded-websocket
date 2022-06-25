#![allow(dead_code)]

// This is an example implementation of compatibility extension, gladly stolen from futures_lite
// As this is far from being any useful, please do extend this on your own
pub trait CompatExt {
    fn compat(self) -> Compat<Self>
        where
            Self: Sized;
    fn compat_ref(&self) -> Compat<&Self>;
    fn compat_mut(&mut self) -> Compat<&mut Self>;
}

impl<T> CompatExt for T {
    fn compat(self) -> Compat<Self>
        where
            Self: Sized,
    {
        Compat(self)
    }

    fn compat_ref(&self) -> Compat<&Self> {
        Compat(self)
    }

    fn compat_mut(&mut self) -> Compat<&mut Self> {
        Compat(self)
    }
}

pub struct Compat<T>(T);

impl<T> Compat<T> {
    pub fn get_ref(&self) -> &T {
        &self.0
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

#[cfg(feature = "example-tokio")]
pub mod tokio_compat {
    use super::Compat;
    use async_trait::async_trait;
    use embedded_websocket::compat::{AsyncRead, AsyncWrite};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[async_trait]
    impl<T: tokio::io::AsyncRead + Unpin + Send + Sync> AsyncRead<std::io::Error> for Compat<T> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
            AsyncReadExt::read(self.get_mut(), buf).await
        }
    }

    #[async_trait]
    impl<T: tokio::io::AsyncWrite + Unpin + Send + Sync> AsyncWrite<std::io::Error> for Compat<T> {
        async fn write_all(&mut self, buf: &[u8]) -> Result<(), std::io::Error> {
            AsyncWriteExt::write_all(self.get_mut(), buf).await
        }
    }
}

#[cfg(feature = "example-smol")]
pub mod smol_compat {
    use super::Compat;
    use async_trait::async_trait;
    use embedded_websocket::compat::{AsyncRead, AsyncWrite};
    use smol::io::{AsyncReadExt, AsyncWriteExt};

    #[async_trait]
    impl<T: smol::io::AsyncRead + Unpin + Send + Sync> AsyncRead<std::io::Error> for Compat<T> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
            AsyncReadExt::read(self.get_mut(), buf).await
        }
    }

    #[async_trait]
    impl<T: smol::io::AsyncWrite + Unpin + Send + Sync> AsyncWrite<std::io::Error> for Compat<T> {
        async fn write_all(&mut self, buf: &[u8]) -> Result<(), std::io::Error> {
            AsyncWriteExt::write_all(self.get_mut(), buf).await
        }
    }
}

#[cfg(feature = "example-async-std")]
pub mod async_std_compat {
    use super::Compat;
    use async_std::io::{ReadExt, WriteExt};
    use async_trait::async_trait;
    use embedded_websocket::compat::{AsyncRead, AsyncWrite};

    #[async_trait]
    impl<T: async_std::io::Read + Unpin + Send + Sync> AsyncRead<std::io::Error> for Compat<T> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
            ReadExt::read(self.get_mut(), buf).await
        }
    }

    #[async_trait]
    impl<T: async_std::io::Write + Unpin + Send + Sync> AsyncWrite<std::io::Error> for Compat<T> {
        async fn write_all(&mut self, buf: &[u8]) -> Result<(), std::io::Error> {
            WriteExt::write_all(self.get_mut(), buf).await
        }
    }
}
