// This module helps you work with the websocket library using a stream of data rather than using it as a raw codec.
// This is the most common use case when working with websockets and is recommended due to the hand shaky nature of
// the protocol as well as the fact that an input buffer can contain multiple websocket frames or maybe only a fragment of one.
// This module allows you to work with discrete websocket frames rather than the multiple fragments you read off a stream.
// NOTE: if you are using the standard library then you can use the built in Read and Write traits from std otherwise
//       you have to implement the Read and Write traits specified below

use crate::{
    framer_core::{Decoded, FramerCore},
    WebSocket, WebSocketCloseStatusCode, WebSocketContext, WebSocketOptions,
    WebSocketSendMessageType, WebSocketState, WebSocketSubProtocol, WebSocketType,
};
use core::str::Utf8Error;
use rand_core::RngCore;

// automagically implement the Stream trait for TcpStream if we are using the standard library
// if you were using no_std you would have to implement your own stream
#[cfg(feature = "std")]
impl Stream<std::io::Error> for std::net::TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        std::io::Read::read(self, buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), std::io::Error> {
        std::io::Write::write_all(self, buf)
    }
}

pub trait Stream<E> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, E>;
    fn write_all(&mut self, buf: &[u8]) -> Result<(), E>;
}

pub enum ReadResult<'a> {
    Binary(&'a [u8]),
    Text(&'a str),
    Pong(&'a [u8]),
    Closed,
}

#[derive(Debug)]
pub enum FramerError<E> {
    Io(E),
    FrameTooLarge(usize),
    Utf8(Utf8Error),
    HttpHeader(httparse::Error),
    WebSocket(crate::Error),
}

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
    pub fn connect<E>(
        &mut self,
        stream: &mut impl Stream<E>,
        websocket_options: &WebSocketOptions,
    ) -> Result<Option<WebSocketSubProtocol>, FramerError<E>> {
        connect(&mut self.core, stream, websocket_options)
    }
}

impl<'a, TRng> Framer<'a, TRng, crate::Server>
where
    TRng: RngCore,
{
    pub fn accept<E>(
        &mut self,
        stream: &mut impl Stream<E>,
        websocket_context: &WebSocketContext,
    ) -> Result<(), FramerError<E>> {
        let len = self.core.encode_accept(websocket_context)?;
        stream
            .write_all(self.core.written(len))
            .map_err(FramerError::Io)?;
        Ok(())
    }
}

impl<'a, TRng, TWebSocketType> Framer<'a, TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    // read and write buffers are usually quite small (4KB) and can be smaller
    // than the frame buffer but use whatever is is appropriate for your stream
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

    pub fn state(&self) -> WebSocketState {
        self.core.state()
    }

    // calling close on a websocket that has already been closed by the other party has no effect
    pub fn close<E>(
        &mut self,
        stream: &mut impl Stream<E>,
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
    ) -> Result<(), FramerError<E>> {
        let len = self.core.encode_close(close_status, status_description)?;
        stream
            .write_all(self.core.written(len))
            .map_err(FramerError::Io)?;
        Ok(())
    }

    pub fn write<E>(
        &mut self,
        stream: &mut impl Stream<E>,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        frame_buf: &[u8],
    ) -> Result<(), FramerError<E>> {
        let len = self
            .core
            .encode_write(message_type, end_of_message, frame_buf)?;
        stream
            .write_all(self.core.written(len))
            .map_err(FramerError::Io)?;
        Ok(())
    }

    // frame_buf should be large enough to hold an entire websocket text frame
    // this function will block until it has recieved a full websocket frame.
    // It will wait until the last fragmented frame has arrived.
    pub fn read<'b, E>(
        &mut self,
        stream: &mut impl Stream<E>,
        frame_buf: &'b mut [u8],
    ) -> Result<ReadResult<'b>, FramerError<E>> {
        read(&mut self.core, stream, frame_buf)
    }
}

pub(crate) fn connect<TRng: RngCore, E>(
    core: &mut FramerCore<'_, TRng, crate::Client>,
    stream: &mut impl Stream<E>,
    websocket_options: &WebSocketOptions,
) -> Result<Option<WebSocketSubProtocol>, FramerError<E>> {
    let (len, web_socket_key) = core.encode_connect(websocket_options)?;
    stream
        .write_all(core.written(len))
        .map_err(FramerError::Io)?;

    loop {
        // read the response from the server and check it to complete the opening handshake
        let received_size = stream
            .read(core.connect_read_space()?)
            .map_err(FramerError::Io)?;
        if received_size == 0 {
            return Err(FramerError::WebSocket(crate::Error::HttpHeaderIncomplete));
        }
        core.commit_read(received_size);

        if let Some(sub_protocol) = core.decode_connect_response(&web_socket_key)? {
            return Ok(sub_protocol);
        }
    }
}

pub(crate) fn read<'b, TRng: RngCore, TWebSocketType: WebSocketType, E>(
    core: &mut FramerCore<'_, TRng, TWebSocketType>,
    stream: &mut impl Stream<E>,
    frame_buf: &'b mut [u8],
) -> Result<ReadResult<'b>, FramerError<E>> {
    loop {
        match core.decode(frame_buf)? {
            Decoded::NeedMoreData => {
                let len = stream.read(core.read_space()?).map_err(FramerError::Io)?;
                if len == 0 {
                    return Ok(ReadResult::Closed);
                }
                core.commit_read(len);
            }
            Decoded::SendThenContinue(len) => {
                stream
                    .write_all(core.written(len))
                    .map_err(FramerError::Io)?;
            }
            Decoded::SendThenClosed(len) => {
                stream
                    .write_all(core.written(len))
                    .map_err(FramerError::Io)?;
                return Ok(ReadResult::Closed);
            }
            Decoded::Message(message) => return message.into_read_result(frame_buf),
        }
    }
}

// ************************************************************************************************
// **************************************** TESTS *************************************************
// ************************************************************************************************

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{WebSocketClient, WebSocketOpCode, WebSocketServer};

    struct DummyStream {
        pub read_buf: Vec<u8>, // starts off populated
        pub read_cursor: usize,
        pub write_buf: Vec<u8>, // starts off empty
        pub max_read_len: usize,
    }

    impl DummyStream {
        pub fn new(read_buf: Vec<u8>) -> Self {
            Self {
                read_buf,
                read_cursor: 0,
                write_buf: Vec::new(),
                max_read_len: usize::MAX,
            }
        }

        pub fn one_byte_per_read(read_buf: Vec<u8>) -> Self {
            Self {
                max_read_len: 1,
                ..Self::new(read_buf)
            }
        }
    }

    impl<E> Stream<E> for DummyStream {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, E> {
            let len = buf
                .len()
                .min(self.read_buf.len() - self.read_cursor)
                .min(self.max_read_len);
            buf[..len].copy_from_slice(&self.read_buf[self.read_cursor..self.read_cursor + len]);
            self.read_cursor += len;
            Ok(len)
        }

        fn write_all(&mut self, buf: &[u8]) -> Result<(), E> {
            self.write_buf.extend_from_slice(buf);
            Ok(())
        }
    }

    #[test]
    fn fragmented_frames() {
        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );

        let mut frame_buf = vec![0; 1024];
        let frames = get_fragmented_frames();
        let mut stream = DummyStream::new(frames);
        let frame = client_framer
            .read::<()>(&mut stream, &mut frame_buf)
            .unwrap();
        match frame {
            ReadResult::Text(x) => {
                assert_eq!(x, "hello world!")
            }
            _ => panic!("expected text frame"),
        };
    }

    #[test]
    fn fragmented_frames_with_control_frames() {
        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );

        let mut frame_buf = vec![0; 1024];
        let frames = get_fragmented_frames_with_ping();
        let mut stream = DummyStream::new(frames);
        let frame = client_framer
            .read::<()>(&mut stream, &mut frame_buf)
            .unwrap();
        match frame {
            ReadResult::Text(x) => {
                assert_eq!(x, "hello world!")
            }
            _ => panic!("expected text frame"),
        };
    }

    #[test]
    fn fragmented_frames_with_control_frames_one_byte_per_read() {
        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );

        let mut frame_buf = vec![0; 1024];
        let mut stream = DummyStream::one_byte_per_read(get_fragmented_frames_with_ping());
        let frame = client_framer
            .read::<()>(&mut stream, &mut frame_buf)
            .unwrap();
        match frame {
            ReadResult::Text(x) => {
                assert_eq!(x, "hello world!")
            }
            _ => panic!("expected text frame"),
        };
        // unmasked pong with an empty payload
        assert_eq!(&stream.write_buf[..2], &[0x8A, 0x80]);
    }

    #[test]
    fn multiple_messages_in_one_read() {
        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );

        let mut frames = get_fragmented_frames();
        frames.extend(get_fragmented_frames_with_ping());
        let mut stream = DummyStream::new(frames);
        let mut frame_buf = vec![0; 1024];
        for _ in 0..2 {
            match client_framer
                .read::<()>(&mut stream, &mut frame_buf)
                .unwrap()
            {
                ReadResult::Text(x) => assert_eq!(x, "hello world!"),
                _ => panic!("expected text frame"),
            }
        }
    }

    #[test]
    fn frames_received_with_handshake_response() {
        use rand::{rngs::StdRng, SeedableRng};

        let websocket_options = WebSocketOptions {
            path: "/chat",
            host: "localhost",
            origin: "http://localhost",
            sub_protocols: None,
            additional_headers: None,
        };
        let mut scratch = vec![0; 1024];
        let (_, web_socket_key) = WebSocketClient::new_client(StdRng::seed_from_u64(1))
            .client_connect(&websocket_options, &mut scratch)
            .unwrap();
        let mut ws_server = WebSocketServer::new_server();
        let len = ws_server
            .server_accept(&web_socket_key, None, &mut scratch)
            .unwrap();
        let mut received = scratch[..len].to_vec();
        received.extend(get_fragmented_frames());

        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(StdRng::seed_from_u64(1));
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );
        let mut stream = DummyStream::new(received);
        client_framer
            .connect::<()>(&mut stream, &websocket_options)
            .unwrap();

        let mut frame_buf = vec![0; 1024];
        match client_framer
            .read::<()>(&mut stream, &mut frame_buf)
            .unwrap()
        {
            ReadResult::Text(x) => assert_eq!(x, "hello world!"),
            _ => panic!("expected text frame"),
        }
    }

    #[test]
    fn connect_fails_when_stream_ends_during_handshake() {
        let mut read_buf = vec![0; 1024];
        let mut write_buf = vec![0; 1024];
        let mut read_cursor = 0;
        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        let mut client_framer = Framer::new(
            &mut read_buf,
            &mut read_cursor,
            &mut write_buf,
            &mut ws_client,
        );
        let websocket_options = WebSocketOptions {
            path: "/chat",
            host: "localhost",
            origin: "http://localhost",
            sub_protocols: None,
            additional_headers: None,
        };
        let mut stream = DummyStream::new(b"HTTP/1.1 101 Switching".to_vec());
        let result = client_framer.connect::<()>(&mut stream, &websocket_options);
        assert!(matches!(
            result,
            Err(FramerError::WebSocket(crate::Error::HttpHeaderIncomplete))
        ));
    }

    fn get_fragmented_frames() -> Vec<u8> {
        // returns a buffer with 3 websocket frames.
        // A fragmented text frame
        //   followed by a text continuation frame
        //   followed by a final text continuation frame
        // "hello" + " world" + "!"
        let mut messages = Vec::new();
        let mut scratch = vec![0u8; 1024];
        let mut ws_server = WebSocketServer::new_server();
        let len = ws_server
            .write_frame(
                "hello".as_bytes(),
                &mut scratch,
                WebSocketOpCode::TextFrame,
                false,
            )
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);
        let len = ws_server
            .write_frame(
                " world".as_bytes(),
                &mut scratch,
                WebSocketOpCode::ContinuationFrame,
                false,
            )
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);

        let len = ws_server
            .write_frame(
                "!".as_bytes(),
                &mut scratch,
                WebSocketOpCode::ContinuationFrame,
                true,
            )
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);
        messages
    }

    fn get_fragmented_frames_with_ping() -> Vec<u8> {
        // returns a buffer with 3 websocket frames.
        // A fragmented text frame
        //   followed by a Ping control message
        //   followed by a final text continuation frame
        // "hello" + Ping + " world!"
        let mut messages = Vec::new();
        let mut scratch = vec![0u8; 1024];
        let mut ws_server = WebSocketServer::new_server();
        let len = ws_server
            .write_frame(
                "hello".as_bytes(),
                &mut scratch,
                WebSocketOpCode::TextFrame,
                false,
            )
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);
        let len = ws_server
            .write_frame(b"", &mut scratch, WebSocketOpCode::Ping, true)
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);
        let len = ws_server
            .write_frame(
                " world!".as_bytes(),
                &mut scratch,
                WebSocketOpCode::ContinuationFrame,
                true,
            )
            .unwrap();
        messages.extend_from_slice(&scratch[..len]);
        messages
    }
}
