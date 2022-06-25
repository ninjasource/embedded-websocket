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
    framer::{Framer, FramerError, ReadResult},
    WebSocketClient, WebSocketCloseStatusCode, WebSocketOptions, WebSocketSendMessageType,
};

cfg_if::cfg_if! {
    if #[cfg(feature = "tokio")] {
        use tokio::net::TcpStream;
    } else if #[cfg(feature = "smol")] {
        use smol::net::TcpStream;
    } else if #[cfg(feature = "async-std")] {
        use async_std::net::TcpStream;
    }
}

#[cfg_attr(feature = "async-std", async_std::main)]
#[cfg_attr(feature = "tokio", tokio::main)]
#[cfg_attr(feature = "smol", smol_potat::main)]
async fn main() -> Result<(), FramerError> {
    // open a TCP stream to localhost port 1337
    let address = "127.0.0.1:1337";
    println!("Connecting to: {}", address);
    let mut stream = TcpStream::connect(address)
        .await
        .map_err(anyhow::Error::new)
        .map_err(FramerError::Io)?;
    println!("Connected.");

    let mut read_buf = [0; 4000];
    let mut read_cursor = 0;
    let mut write_buf = [0; 4000];
    let mut frame_buf = [0; 4000];
    let mut websocket = WebSocketClient::new_client(rand::thread_rng());

    // initiate a websocket opening handshake
    let websocket_options = WebSocketOptions {
        path: "/chat",
        host: "localhost",
        origin: "http://localhost:1337",
        sub_protocols: None,
        additional_headers: None,
    };

    let mut framer = Framer::new(
        &mut read_buf,
        &mut read_cursor,
        &mut write_buf,
        &mut websocket,
    );
    framer
        .connect_async(&mut stream, &websocket_options)
        .await?;

    let message = "Hello, World!";
    framer
        .write_async(
            &mut stream,
            WebSocketSendMessageType::Text,
            true,
            message.as_bytes(),
        )
        .await?;

    while let ReadResult::Text(s) = framer.read_async(&mut stream, &mut frame_buf).await? {
        println!("Received: {}", s);

        // close the websocket after receiving the first reply
        framer
            .close_async(&mut stream, WebSocketCloseStatusCode::NormalClosure, None)
            .await?;
        println!("Sent close handshake");
    }

    println!("Connection closed");
    Ok(())
}