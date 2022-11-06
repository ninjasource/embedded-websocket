use std::error::Error;
use std::io;

use bytes::{BufMut, BytesMut};
use tokio::net::TcpStream;
use tokio_util::codec::{Decoder, Encoder, Framed};

use embedded_websocket::{
    framer_async::{Framer, FramerError, ReadResult},
    WebSocketClient, WebSocketCloseStatusCode, WebSocketOptions, WebSocketSendMessageType,
};

struct MyCodec {}

impl MyCodec {
    fn new() -> Self {
        MyCodec {}
    }
}

impl Decoder for MyCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
        if !buf.is_empty() {
            let len = buf.len();
            Ok(Some(buf.split_to(len)))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<&[u8]> for MyCodec {
    type Error = io::Error;

    fn encode(&mut self, data: &[u8], buf: &mut BytesMut) -> Result<(), io::Error> {
        buf.reserve(data.len());
        buf.put(data);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), FramerError<impl Error>> {
    // Connect to a peer
    let address = "127.0.0.1:1337";
    let mut buffer = [0u8; 4000];
    let stream = TcpStream::connect(address).await.map_err(FramerError::Io)?;
    let codec = MyCodec::new();
    let mut stream = Framed::new(stream, codec);
    let websocket = WebSocketClient::new_client(rand::thread_rng());

    // initiate a websocket opening handshake
    let websocket_options = WebSocketOptions {
        path: "/chat",
        host: "localhost",
        origin: "http://localhost:1337",
        sub_protocols: None,
        additional_headers: None,
    };

    let mut framer = Framer::new(websocket);

    framer
        .connect(&mut stream, &mut buffer, &websocket_options)
        .await?;

    println!("ws handshake complete");

    framer
        .write(
            &mut stream,
            &mut buffer,
            WebSocketSendMessageType::Text,
            true,
            "Hello, world".as_bytes(),
        )
        .await?;

    println!("sent message");

    while let Some(read_result) = framer.read(&mut stream, &mut buffer).await {
        let read_result = read_result?;
        match read_result {
            ReadResult::Text(text) => {
                println!("received text: {text}");

                framer
                    .close(
                        &mut stream,
                        &mut buffer,
                        WebSocketCloseStatusCode::NormalClosure,
                        None,
                    )
                    .await?
            }
            _ => { // ignore other kinds of messages
            }
        }
    }

    println!("closed");
    Ok(())
}
