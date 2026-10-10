//! A websocket framer for async streams that implement the [`embedded_io_async`] traits.
//!
//! The [`Framer`] reads and writes whole websocket messages and has the same API as
//! [`crate::framer::Framer`]. It handles stream reads that return a partial frame, a partial
//! frame header or multiple frames at once.
//!
//! Any stream that implements [`embedded_io_async::Read`] and [`embedded_io_async::Write`]
//! works, for example the `TcpSocket` of `embassy-net`.

use embedded_io_async::{Read, Write};
use rand_core::RngCore;

pub use crate::framer::{FramerError, ReadResult};
use crate::{
    framer_core::{Decoded, FramerCore},
    WebSocket, WebSocketCloseStatusCode, WebSocketContext, WebSocketOptions,
    WebSocketSendMessageType, WebSocketState, WebSocketSubProtocol, WebSocketType,
};

/// Reads and writes whole websocket messages over a async stream.
///
/// # Examples
///
/// ```no_run
/// use embedded_websocket::{
///     framer_embedded_io_async::{Framer, FramerError, ReadResult},
///     WebSocketClient, WebSocketOptions, WebSocketSendMessageType,
/// };
///
/// async fn hello<S: embedded_io_async::Read + embedded_io_async::Write>(
///     stream: &mut S,
/// ) -> Result<(), FramerError<S::Error>> {
///     let mut read_buf = [0; 1024];
///     let mut read_cursor = 0;
///     let mut write_buf = [0; 1024];
///     let mut frame_buf = [0; 1024];
///     let mut websocket = WebSocketClient::new_client(rand::thread_rng());
///     let mut framer = Framer::new(
///         &mut read_buf,
///         &mut read_cursor,
///         &mut write_buf,
///         &mut websocket,
///     );
///
///     let websocket_options = WebSocketOptions {
///         path: "/chat",
///         host: "localhost",
///         origin: "http://localhost",
///         sub_protocols: None,
///         additional_headers: None,
///     };
///     framer.connect(stream, &websocket_options).await?;
///     framer
///         .write(stream, WebSocketSendMessageType::Text, true, b"hello")
///         .await?;
///     if let ReadResult::Text(text) = framer.read(stream, &mut frame_buf).await? {
///         assert_eq!("hello", text);
///     }
///     Ok(())
/// }
/// ```
pub struct Framer<'a, TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    core: FramerCore<'a, TRng, TWebSocketType>,
}

impl<'a, TRng> Framer<'a, TRng, crate::Client>
where
    TRng: RngCore,
{
    /// Performs the client side of the opening handshake.
    ///
    /// Returns the sub protocol the server chose from
    /// [`WebSocketOptions::sub_protocols`], if any.
    ///
    /// # Errors
    ///
    /// * [`FramerError::Io`] if reading from or writing to the stream fails.
    /// * [`FramerError::WebSocket`] with [`Error::HttpHeaderIncomplete`] if the stream ends or
    ///   the read buffer is full before the HTTP response header is complete.
    /// * [`FramerError::WebSocket`] if the server rejects the opening handshake.
    ///
    /// [`Error::HttpHeaderIncomplete`]: crate::Error::HttpHeaderIncomplete
    pub async fn connect<S>(
        &mut self,
        stream: &mut S,
        websocket_options: &WebSocketOptions<'_>,
    ) -> Result<Option<WebSocketSubProtocol>, FramerError<S::Error>>
    where
        S: Read + Write,
    {
        let (len, web_socket_key) = self.core.encode_connect(websocket_options)?;
        write_all(stream, self.core.written(len)).await?;

        loop {
            let received_size = stream
                .read(self.core.connect_read_space()?)
                .await
                .map_err(FramerError::Io)?;
            if received_size == 0 {
                return Err(FramerError::WebSocket(crate::Error::HttpHeaderIncomplete));
            }
            self.core.commit_read(received_size);

            if let Some(sub_protocol) = self.core.decode_connect_response(&web_socket_key)? {
                return Ok(sub_protocol);
            }
        }
    }
}

impl<'a, TRng> Framer<'a, TRng, crate::Server>
where
    TRng: RngCore,
{
    /// Performs the server side of the opening handshake.
    ///
    /// Read the HTTP request header with [`crate::read_http_header`] first and pass the
    /// resulting context.
    ///
    /// # Errors
    ///
    /// * [`FramerError::Io`] if writing to the stream fails.
    /// * [`FramerError::WebSocket`] if the websocket is already open.
    pub async fn accept<S>(
        &mut self,
        stream: &mut S,
        websocket_context: &WebSocketContext,
    ) -> Result<(), FramerError<S::Error>>
    where
        S: Write,
    {
        let len = self.core.encode_accept(websocket_context)?;
        write_all(stream, self.core.written(len)).await
    }
}

impl<'a, TRng, TWebSocketType> Framer<'a, TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    /// Creates a framer.
    ///
    /// `read_buf` holds bytes read from the stream. It must fit the HTTP response header of the
    /// opening handshake and any control frame (up to 131 bytes).
    ///
    /// `read_cursor` is the number of bytes at the start of `read_buf` that are not decoded
    /// yet. Use 0 for a new connection. A framer created with the same `read_buf` and
    /// `read_cursor` continues where the previous one stopped.
    ///
    /// `write_buf` holds encoded frames. It must fit the largest payload you write plus the
    /// frame header (2 to 14 bytes).
    pub fn new(
        read_buf: &'a mut [u8],
        read_cursor: &'a mut usize,
        write_buf: &'a mut [u8],
        websocket: &'a mut WebSocket<TRng, TWebSocketType>,
    ) -> Self {
        Self {
            core: FramerCore::new(read_buf, read_cursor, write_buf, websocket),
        }
    }

    /// Returns the state of the websocket.
    pub fn state(&self) -> WebSocketState {
        self.core.state()
    }

    /// Initiates a close handshake.
    ///
    /// Has no effect if the other party already closed the websocket.
    ///
    /// # Errors
    ///
    /// * [`FramerError::Io`] if writing to the stream fails.
    /// * [`FramerError::WebSocket`] if the websocket is not open.
    pub async fn close<S>(
        &mut self,
        stream: &mut S,
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
    ) -> Result<(), FramerError<S::Error>>
    where
        S: Write,
    {
        let len = self.core.encode_close(close_status, status_description)?;
        write_all(stream, self.core.written(len)).await
    }

    /// Writes `frame_buf` to the stream as one websocket frame.
    ///
    /// Set `end_of_message` to `false` to send a message in multiple frames and to `true` for
    /// the last frame.
    ///
    /// # Errors
    ///
    /// * [`FramerError::Io`] if writing to the stream fails.
    /// * [`FramerError::WebSocket`] if the websocket is not open or the frame does not fit into
    ///   the write buffer.
    pub async fn write<S>(
        &mut self,
        stream: &mut S,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        frame_buf: &[u8],
    ) -> Result<(), FramerError<S::Error>>
    where
        S: Write,
    {
        let len = self
            .core
            .encode_write(message_type, end_of_message, frame_buf)?;
        write_all(stream, self.core.written(len)).await
    }

    /// Reads from the stream until a whole message arrived.
    ///
    /// Answers ping messages with a pong message and completes a close handshake the other party
    /// initiated. Returns [`ReadResult::Closed`] when the close handshake is complete or the
    /// stream ended.
    ///
    /// # Errors
    ///
    /// * [`FramerError::Io`] if reading from or writing to the stream fails.
    /// * [`FramerError::FrameTooLarge`] if the message does not fit into `frame_buf`, or a frame
    ///   header or control frame does not fit into the read buffer.
    /// * [`FramerError::Utf8`] if a text message is not valid UTF-8.
    /// * [`FramerError::WebSocket`] if the other party violates the websocket protocol.
    pub async fn read<'b, S>(
        &mut self,
        stream: &mut S,
        frame_buf: &'b mut [u8],
    ) -> Result<ReadResult<'b>, FramerError<S::Error>>
    where
        S: Read + Write,
    {
        loop {
            match self.core.decode(frame_buf)? {
                Decoded::NeedMoreData => {
                    let len = stream
                        .read(self.core.read_space()?)
                        .await
                        .map_err(FramerError::Io)?;
                    if len == 0 {
                        return Ok(ReadResult::Closed);
                    }
                    self.core.commit_read(len);
                }
                Decoded::SendThenContinue(len) => {
                    write_all(stream, self.core.written(len)).await?;
                }
                Decoded::SendThenClosed(len) => {
                    write_all(stream, self.core.written(len)).await?;
                    return Ok(ReadResult::Closed);
                }
                Decoded::Message(message) => return message.into_read_result(frame_buf),
            }
        }
    }
}

async fn write_all<S: Write>(stream: &mut S, buf: &[u8]) -> Result<(), FramerError<S::Error>> {
    stream.write_all(buf).await.map_err(FramerError::Io)?;
    stream.flush().await.map_err(FramerError::Io)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::{vec, vec::Vec};

    use super::*;
    use crate::{WebSocketClient, WebSocketOpCode, WebSocketServer};

    struct OneBytePerReadStream {
        read_buf: Vec<u8>,
        read_cursor: usize,
        write_buf: Vec<u8>,
    }

    impl embedded_io_async::ErrorType for OneBytePerReadStream {
        type Error = core::convert::Infallible;
    }

    impl Read for OneBytePerReadStream {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() || self.read_cursor == self.read_buf.len() {
                return Ok(0);
            }
            buf[0] = self.read_buf[self.read_cursor];
            self.read_cursor += 1;
            Ok(1)
        }
    }

    impl Write for OneBytePerReadStream {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.write_buf.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn fragmented_frames_with_control_frames_one_byte_per_read() {
        let mut frames = Vec::new();
        let mut scratch = vec![0u8; 1024];
        let mut ws_server = WebSocketServer::new_server();
        for (payload, op_code, fin) in [
            ("hello", WebSocketOpCode::TextFrame, false),
            ("", WebSocketOpCode::Ping, true),
            (" world!", WebSocketOpCode::ContinuationFrame, true),
        ] {
            let len = ws_server
                .write_frame(payload.as_bytes(), &mut scratch, op_code, fin)
                .unwrap();
            frames.extend_from_slice(&scratch[..len]);
        }

        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );
        let mut stream = OneBytePerReadStream {
            read_buf: frames,
            read_cursor: 0,
            write_buf: Vec::new(),
        };

        let mut frame_buf = vec![0; 1024];
        match framer.read(&mut stream, &mut frame_buf).await.unwrap() {
            ReadResult::Text(x) => assert_eq!(x, "hello world!"),
            _ => panic!("expected text frame"),
        }
        // unmasked pong with an empty payload
        assert_eq!(&stream.write_buf[..2], &[0x8A, 0x80]);

        assert!(matches!(
            framer.read(&mut stream, &mut frame_buf).await.unwrap(),
            ReadResult::Closed
        ));
    }
}
