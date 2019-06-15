// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Demo websocket client connecting to localhost port 1337.
// This will initiate a websocket connection to path /chat. The demo sends a simple "Hello, World!"
// message and expects an echo of the same message as a reply.
// It will then initiate a close handshake, wait for a close response from the server,
// and terminate the connection.
// Note that we are using the standard library in the demo but the websocket library remains no_std

use embedded_websockets as ws;
use embedded_websockets::WebSocketOptions;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::Utf8Error;
use ws::{
    WebSocket, WebSocketCloseStatusCode, WebSocketReceiveMessageType, WebSocketSendMessageType,
};

#[derive(Debug)]
enum WebClientError {
    Io(std::io::Error),
    WebSocket(ws::Error),
    Utf8Error,
}

type Result<T> = std::result::Result<T, WebClientError>;

impl From<std::io::Error> for WebClientError {
    fn from(err: std::io::Error) -> WebClientError {
        WebClientError::Io(err)
    }
}

impl From<ws::Error> for WebClientError {
    fn from(err: ws::Error) -> WebClientError {
        WebClientError::WebSocket(err)
    }
}

impl From<Utf8Error> for WebClientError {
    fn from(_: Utf8Error) -> WebClientError {
        WebClientError::Utf8Error
    }
}

fn write_all(stream: &mut TcpStream, buffer: &[u8]) -> Result<()> {
    let mut from = 0;
    loop {
        let bytes_sent = stream.write(&buffer[from..])?;
        from = from + bytes_sent;

        if from == buffer.len() {
            break;
        }
    }

    stream.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    // open a TCP stream to localhost port 1337
    let address = "127.0.0.1:1337";
    println!("Connecting to: {}", address);
    let mut stream = TcpStream::connect(address)?;
    println!("Connected.");

    let mut buffer1: [u8; 4000] = [0; 4000];
    let mut buffer2: [u8; 4000] = [0; 4000];
    let mut ws_client = WebSocket::new_client(rand::thread_rng());

    // initiate a websocket opening handshake
    let websocket_options = WebSocketOptions {
        path: "/chat",
        host: "localhost",
        port: 1337,
        sub_protocols: None,
        additional_headers: None,
    };
    let (len, web_socket_key) = ws_client.client_connect(&websocket_options, &mut buffer1)?;
    println!("Sending opening handshake: {} bytes", len);
    write_all(&mut stream, &buffer1[..len])?;

    // read the response from the server and check it to complete the opening handshake
    let received_size = stream.read(&mut buffer1)?;
    ws_client.client_accept(&web_socket_key, &mut buffer1[..received_size])?;
    println!("Opening handshake completed successfully");

    // send a Text frame to the server
    let message = "Hello, World!";
    let send_size = ws_client.write(
        WebSocketSendMessageType::Text,
        true,
        &message.as_bytes(),
        &mut buffer1,
    )?;
    println!("Sending '{}': {} bytes", message, send_size);
    write_all(&mut stream, &buffer1[..send_size])?;

    // read the response from the server (we expect the server to simply echo the same message back)
    let received_size = stream.read(&mut buffer1)?;
    println!("Received: {} bytes", received_size);
    let ws_result = ws_client.read(&buffer1[..received_size], &mut buffer2)?;

    match ws_result.message_type {
        WebSocketReceiveMessageType::Text => {
            let s = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?;
            println!("Text reply from server: {}", s);
        }
        _ => {
            let s = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?;
            println!(
                "Unexpected response from server: {:?} {} bytes: {}",
                ws_result.message_type, ws_result.num_bytes_to, s
            );
        }
    }

    // initiate a close handshake
    let send_size = ws_client.close(WebSocketCloseStatusCode::NormalClosure, None, &mut buffer2)?;
    stream.write(&buffer2[..send_size])?;
    println!("Sent close handshake");

    // read the reply from the server to complete the close handshake
    let received_size = stream.read(&mut buffer1)?;
    println!("Received: {} bytes", received_size);
    let ws_result = ws_client.read(&buffer1[..received_size], &mut buffer2)?;
    match ws_result.message_type {
        WebSocketReceiveMessageType::CloseCompleted => {
            println!("Completed close handshake");
        }
        _ => {
            println!(
                "Received unexpected message: {:?} {} bytes",
                ws_result.message_type, ws_result.num_bytes_to
            );
        }
    }

    Ok(())
}
