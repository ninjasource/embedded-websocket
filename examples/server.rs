// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Demo websocket server that listens on localhost port 1337.
// If accessed from a browser it will return a web page that will automatically attempt to
// open a websocket connection to itself. Alternatively, the websocket_client can be used to
// open a websocket connection directly. The server will echo all Text and Ping messages back to
// the client as well as responding to any opening and closing handshakes.
// Note that we are using the standard library in the demo but the websocket library remains no_std

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::str::Utf8Error;
use std::thread;

use embedded_websockets as ws;
use ws::{
    HttpHeader, WebSocketReceiveMessageType, WebSocketSendMessageType, WebSocketServer,
    WebSocketState,
};

type Result<T> = std::result::Result<T, WebServerError>;

#[derive(Debug)]
enum WebServerError {
    Io(std::io::Error),
    WebSocket(ws::Error),
    Utf8Error,
}

impl From<std::io::Error> for WebServerError {
    fn from(err: std::io::Error) -> WebServerError {
        WebServerError::Io(err)
    }
}

impl From<ws::Error> for WebServerError {
    fn from(err: ws::Error) -> WebServerError {
        WebServerError::WebSocket(err)
    }
}

impl From<Utf8Error> for WebServerError {
    fn from(_: Utf8Error) -> WebServerError {
        WebServerError::Utf8Error
    }
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:1337")?;

    // accept connections and process them serially
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| match handle_client(stream) {
                    Ok(()) => println!("Connection closed"),
                    Err(e) => println!("Error: {:?}", e),
                });
            }
            Err(e) => println!("Failed to establish a connection: {}", e),
        }
    }

    Ok(())
}

fn handle_client(mut stream: TcpStream) -> Result<()> {
    let mut buffer1: [u8; 3000] = [0; 3000];
    let mut buffer2: [u8; 3000] = [0; 3000];
    let mut web_socket = WebSocketServer::new_server();
    let mut num_bytes = 0;

    // read until the stream is closed (zero bytes read from the stream)
    loop {
        if num_bytes >= buffer1.len() {
            println!("Not a valid http request or buffer too small");
            return Ok(());
        }

        num_bytes = num_bytes + stream.read(&mut buffer1[num_bytes..])?;
        println!("Received {} bytes", num_bytes);
        if num_bytes == 0 {
            return Ok(());
        }

        if web_socket.state == WebSocketState::Open {
            // if the tcp stream has already been upgraded to a websocket connection
            if !web_socket_read(&mut web_socket, &mut stream, &mut buffer1, &mut buffer2, num_bytes)? {
                println!("Websocket closed");
                return Ok(());
            }
            num_bytes = 0;
        } else {
            // assume that the client has sent us an http request. Since we may not read the
            // header all in one go we need to check for HttpHeaderIncomplete and continue reading
            if !match ws::read_http_header(&buffer1[..num_bytes]) {
                Ok(http_header) => {
                    println!("Http header read");
                    num_bytes = 0;
                    respond_to_http_request(http_header, &mut web_socket, &mut buffer2, &mut stream)
                }
                Err(ws::Error::HttpHeaderIncomplete) => Ok(true),
                Err(e) => Err(WebServerError::WebSocket(e)),
            }? {
                return Ok(());
            }
        }
    }
}

// returns true to keep the connection open
fn respond_to_http_request(
    http_header: HttpHeader,
    web_socket: &mut WebSocketServer,
    buffer2: &mut [u8],
    stream: &mut TcpStream,
) -> Result<bool> {
    if let Some(websocket_context) = http_header.websocket_context {
        // this is a web socket upgrade request
        println!("Received websocket upgrade request");
        let to_send =
            web_socket.server_accept(&websocket_context.sec_websocket_key, None, buffer2)?;
        write_to_stream(stream, &buffer2[..to_send])?;
        Ok(true)
    } else {
        println!("Received file request: {}", http_header.path.as_str());

        // this is a regular http request
        match http_header.path.as_str() {
            "/" => {
                write_to_stream(stream, &ROOT_HTML.as_bytes())?;
            }
            _ => {
                let html =
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                write_to_stream(stream, &html.as_bytes())?;
            }
        };
        Ok(false)
    }
}

fn write_to_stream(stream: &mut TcpStream, buffer: &[u8]) -> Result<()> {
    let mut start = 0;
    loop {
        let bytes_sent = stream.write(&buffer[start..])?;
        start += bytes_sent;

        if start == buffer.len() {
            println!("Sent {} bytes", buffer.len());
            return Ok(());
        }
    }
}

fn web_socket_read(
    web_socket: &mut WebSocketServer,
    stream: &mut TcpStream,
    tcp_buffer: &mut [u8],
    ws_buffer: &mut [u8],
    num_bytes: usize,
) -> Result<bool> {
    let ws_result = web_socket.read(&tcp_buffer[..num_bytes], ws_buffer)?;
    match ws_result.message_type {
        WebSocketReceiveMessageType::Text => {
            let s = std::str::from_utf8(&ws_buffer[..ws_result.num_bytes_to])?;
            println!("Received Text: {}", s);
            let to_send = web_socket.write(
                WebSocketSendMessageType::Text,
                true,
                &ws_buffer[..ws_result.num_bytes_to],
                tcp_buffer,
            )?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        }
        WebSocketReceiveMessageType::Binary => {
            // ignored
            Ok(true)
        }
        WebSocketReceiveMessageType::CloseCompleted => Ok(false),
        WebSocketReceiveMessageType::CloseMustReply => {
            let to_send = web_socket.write(
                WebSocketSendMessageType::CloseReply,
                true,
                &ws_buffer[..ws_result.num_bytes_to],
                tcp_buffer,
            )?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        }
        WebSocketReceiveMessageType::Ping => {
            let to_send = web_socket.write(
                WebSocketSendMessageType::Pong,
                true,
                &ws_buffer[..ws_result.num_bytes_to],
                tcp_buffer,
            )?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        }
        WebSocketReceiveMessageType::Pong => {
            println!("Received Pong");
            Ok(true)
        }
    }
}

const ROOT_HTML : &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 2590\r\nConnection: close\r\n\r\n<!doctype html>
<html>
<head>
    <meta content='text/html;charset=utf-8' http-equiv='Content-Type' />
    <meta content='utf-8' http-equiv='encoding' />
    <meta name='viewport' content='width=device-width, initial-scale=0.5, maximum-scale=0.5, user-scalable=0' />
    <meta name='apple-mobile-web-app-capable' content='yes' />
    <meta name='apple-mobile-web-app-status-bar-style' content='black' />
    <title>Web Socket Demo</title>
    <style type='text/css'>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font: 13px Helvetica, Arial; }
        form { background: #000; padding: 3px; position: fixed; bottom: 0; width: 100%; }
        form input { border: 0; padding: 10px; width: 90%; margin-right: .5%; }
        form button { width: 9%; background: rgb(130, 200, 255); border: none; padding: 10px; }
        #messages { list-style-type: none; margin: 0; padding: 0; }
        #messages li { padding: 5px 10px; }
        #messages li:nth-child(odd) { background: #eee; }
    </style>
</head>
<body>
    <ul id='messages'></ul>
    <form action=''>
    <input id='txtBox' autocomplete='off' /><button>Send</button>
    </form>
    <script type='text/javascript' src='http://code.jquery.com/jquery-1.11.1.js' ></script>
    <script type='text/javascript'>
        var CONNECTION;
        window.onload = function () {
            // open the connection to the Web Socket server
            CONNECTION = new WebSocket('ws://localhost:1337/chat');
			// CONNECTION = new WebSocket('ws://' + location.host + ':1337/chat');

            // When the connection is open
            CONNECTION.onopen = function () {
                $('#messages').append($('<li>').text('Connection opened'));
            };

            // when the connection is closed by the server
            CONNECTION.onclose = function () {
                $('#messages').append($('<li>').text('Connection closed'));
            };

            // Log errors
            CONNECTION.onerror = function (e) {
                console.log('An error occured');
            };

            // Log messages from the server
            CONNECTION.onmessage = function (e) {
                $('#messages').append($('<li>').text(e.data));
            };
        };

		$(window).on('beforeunload', function(){
			CONNECTION.close();
		});

        // when we press the Send button, send the text to the server
        $('form').submit(function(){
            CONNECTION.send($('#txtBox').val());
            $('#txtBox').val('');
            return false;
        });
    </script>
</body>
</html>";
