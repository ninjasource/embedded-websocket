use core::result::Result;
// use core::pin::Pin;

pub trait Read<E> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, E>;
}

pub trait Write<E> {
    fn write_all(&mut self, buf: &[u8]) -> Result<(), E>;
}

cfg_if::cfg_if! {
    if #[cfg(feature = "async")] {
        #[async_trait::async_trait]
        pub trait AsyncRead<E> {
            async fn read(&mut self, buf: &mut [u8]) -> Result<usize, E>;
        }

        #[async_trait::async_trait]
        pub trait AsyncWrite<E> {
            async fn write_all(&mut self, buf: &[u8]) -> Result<(), E>;
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        impl Read<std::io::Error> for std::net::TcpStream {
            fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
                std::io::Read::read(self, buf)
            }
        }

        impl Write<std::io::Error> for std::net::TcpStream {
            fn write_all(&mut self, buf: &[u8]) -> Result<(), std::io::Error> {
                std::io::Write::write_all(self, buf)
            }
        }
    }
}
