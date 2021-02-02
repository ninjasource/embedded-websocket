// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Demo websocket client connecting to localhost port 1337.
// This will initiate a websocket connection to path /chat. The demo sends a simple "Hello, World!"
// message and expects an echo of the same message as a reply.
// It will then initiate a close handshake, wait for a close response from the server,
// and terminate the connection.
// Note that we are using the standard library in the demo and making use of the framer helper module
// but the websocket library remains no_std (see client_full for an example without the framer helper module)

use embedded_websocket::{
    framer::{Framer, FramerError},
    WebSocketClient, WebSocketCloseStatusCode, WebSocketOptions, WebSocketSendMessageType,
};
use std::net::TcpStream;

fn main() -> Result<(), FramerError> {
    // open a TCP stream to localhost port 1337
    let address = "127.0.0.1:1337";
    println!("Connecting to: {}", address);
    let mut stream = TcpStream::connect(address)?;
    println!("Connected.");

    let mut read_buf: [u8; 4000] = [0; 4000];
    let mut write_buf: [u8; 4000] = [0; 4000];
    let mut frame_buf: [u8; 4000] = [0; 4000];
    let mut ws_client = WebSocketClient::new_client(rand::thread_rng());

    // initiate a websocket opening handshake
    let websocket_options = WebSocketOptions {
        path: "/chat",
        host: "localhost",
        origin: "http://localhost:1337",
        sub_protocols: None,
        additional_headers: None,
    };

    let mut websocket = Framer::new(&mut read_buf, &mut write_buf, &mut ws_client, &mut stream);
    websocket.connect(&websocket_options)?;

    let message = "Hello, World!";
    websocket.write(WebSocketSendMessageType::Text, true, message.as_bytes())?;

    while let Some(s) = websocket.read_text(&mut frame_buf)? {
        println!("Received: {}", s);

        // close the websocket after receiving the first reply
        websocket.close(WebSocketCloseStatusCode::NormalClosure, None)?;
        println!("Sent close handshake");
    }

    println!("Connection closed");
    Ok(())
}
