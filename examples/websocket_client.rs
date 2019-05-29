// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Demo websocket client connecting to localhost port 1337.
// This will initiate a websocket connection to path /chat. The demo sends a simple "Hello, World!"
// message and expects an echo of the same message as a reply.
// It will then initiate a close handshake, wait for a close response from the server,
// and terminate the connection.
// Note that we are using the standard library in the demo but the websocket library remains no_std

use std::net::{TcpStream};
use std::io::{Read, Write};
use websockets::{WebSocket, WebSocketReceiveMessageType, WebSocketSendMessageType, WebSocketCloseStatusCode};
use std::str::Utf8Error;

#[derive(Debug)]
enum WebClientError {
    Io(std::io::Error),
    WebSocket(websockets::Error),
    Utf8Error,
}

type Result<T> = std::result::Result<T, WebClientError>;

impl From<std::io::Error> for WebClientError {
    fn from(err: std::io::Error) -> WebClientError { WebClientError::Io(err) }
}

impl From<websockets::Error> for WebClientError {
    fn from(err: websockets::Error) -> WebClientError { WebClientError::WebSocket(err )}
}

impl From<Utf8Error> for WebClientError {
    fn from(_: Utf8Error) -> WebClientError { WebClientError::Utf8Error }
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
    let address = "127.0.0.1:1337";
    println!("Connecting to {}...", address);

    let mut stream = TcpStream::connect(address)?;
    println!("Connected.");

    let mut tcp_buffer: [u8; 4000] = [0; 4000];
    let mut ws_buffer: [u8; 4000] = [0; 4000];

    let mut ws_client = WebSocket::new_client();
    let (len, web_socket_key) = ws_client.client_initiate_opening_handshake("/chat", "localhost", "1337", None, None, &mut tcp_buffer)?;

    println!("Sending opening handshake: {} bytes", len);
    write_all(&mut stream, &tcp_buffer[..len])?;

    let received_size = stream.read(&mut tcp_buffer)?;
    ws_client.client_complete_opening_handshake(web_socket_key.as_str(), &mut tcp_buffer[..received_size])?;
    let message = "Hello, World!";
    let send_size = ws_client.write(WebSocketSendMessageType::Text, true, &message.as_bytes(), &mut tcp_buffer)?;
    println!("Sending '{}': {} bytes",message, send_size);

    write_all(&mut stream, &tcp_buffer[..send_size])?;
    let received_size = stream.read(&mut tcp_buffer)?;
    println!("Received: {} bytes", received_size);

    let ws_result = ws_client.read(&tcp_buffer[..received_size], &mut ws_buffer)?;

    match ws_result.message_type {
        WebSocketReceiveMessageType::Text => {
            let s = std::str::from_utf8(&ws_buffer[..ws_result.num_bytes_to])?;
            println!("Reply from server: {}", s);
        }
        _ => {
            let s = std::str::from_utf8(&ws_buffer[..ws_result.num_bytes_to])?;
            println!("Server: {:?} {} bytes: {}", ws_result.message_type, ws_result.num_bytes_to, s);
        }
    }

    let send_size = ws_client.close(WebSocketCloseStatusCode::NormalClosure, None, &mut ws_buffer)?;
    stream.write(&ws_buffer[..send_size])?;
    println!("Sent close handshake");
    let received_size = stream.read(&mut tcp_buffer)?;
    println!("Received: {} bytes", received_size);
    let ws_result = ws_client.read(&tcp_buffer[..received_size], &mut ws_buffer)?;
    match ws_result.message_type {
        WebSocketReceiveMessageType::CloseCompleted => {
            println!("Completed close handshake");
        }
        _ => {
            println!("Received unexpected message: {:?} {} bytes", ws_result.message_type, ws_result.num_bytes_to);
        }
    }

    Ok(())
}