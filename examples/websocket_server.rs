// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// Demo websocket server that listens on localhost port 1337.
// If accessed from a browser it will return a web page that will automatically attempt to
// open a websocket connection to itself. Alternatively, the websocket_client can be used to
// open a websocket connection directly. The server will echo all Text and Ping messages back to
// the client as well as responding to any opening and closing handshakes.
// Note that we are using the standard library in the demo but the websocket library remains no_std

use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use websockets::{WebSocket, WebSocketState,
                 WebSocketReceiveMessageType, WebSocketSendMessageType};
use std::thread;
use std::str::Utf8Error;

type Result<T> = std::result::Result<T, WebServerError>;

#[derive(Debug)]
enum WebServerError {
    Io(std::io::Error),
    WebSocket(websockets::Error),
    Utf8Error,
}

impl From<std::io::Error> for WebServerError {
    fn from(err: std::io::Error) -> WebServerError { WebServerError::Io(err) }
}

impl From<websockets::Error> for WebServerError {
    fn from(err: websockets::Error) -> WebServerError { WebServerError::WebSocket(err )}
}

impl From<Utf8Error> for WebServerError {
    fn from(_: Utf8Error) -> WebServerError { WebServerError::Utf8Error }
}

fn handle_client(mut stream: TcpStream) -> Result<()> {
    let root_html = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: 2590\r\nConnection: close\r\n\r\n<!doctype html>
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

    let mut tcp_buffer: [u8; 3000] = [0; 3000];
    let mut ws_buffer: [u8; 3000] = [0; 3000];

    let mut web_socket = WebSocket::new_server();

    loop {
        let num_bytes = stream.read(&mut tcp_buffer)?;
        println!("Received {} bytes", num_bytes);
        if num_bytes == 0 {
            return Ok(());
        }

        if web_socket.get_state() == WebSocketState::Open {
            if !web_socket_read(&mut web_socket, &mut stream,
                                &mut tcp_buffer, &mut ws_buffer)?
            {
                println!("Websocket closed");
                return Ok(());
            }
        } else {
            let http_header = websockets::read_http_header(&tcp_buffer)?;
            if let Some(websocket_context) = http_header.websocket_context {
                // this is a web socket upgrade request
                println!("Received websocket upgrade request");
                let to_send = web_socket.server_respond_to_opening_handshake(
                    &websocket_context.sec_websocket_key, None, &mut ws_buffer)?;
                write_to_stream(&mut stream, &ws_buffer[..to_send])?;
            } else {
                println!("Received file request: {}", http_header.path.as_str());

                // this is a regular http request
                // write the html to the stream and close the connection by exiting the loop
                match http_header.path.as_str() {
                    "/" => {
                        return write_to_stream(&mut stream, &root_html.as_bytes());
                    }
                    _ => {
                        let html = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        return write_to_stream(&mut stream, &html.as_bytes());
                    }
                };
            }
        }
    }
}

fn write_to_stream(stream: &mut TcpStream, buffer: &[u8]) -> Result<()> {
    let mut start = 0;
    loop {
        let bytes_sent = stream.write(&buffer[start..])?;
        start += bytes_sent;

        if start == buffer.len() {
            println!("Sent {} bytes", buffer.len());
            return Ok(())
        }
    }
}

fn web_socket_read(web_socket: &mut WebSocket, stream : &mut TcpStream, tcp_buffer: &mut [u8],
                   ws_buffer: &mut [u8]) -> Result<bool> {
    let ws_result = web_socket.read(tcp_buffer, ws_buffer)?;
    match ws_result.message_type {
        WebSocketReceiveMessageType::Text => {
            let s = std::str::from_utf8(&ws_buffer[..ws_result.num_bytes_to])?;
            println!("Received Text: {}", s);
            let to_send = web_socket.write(WebSocketSendMessageType::Text, true, &ws_buffer[..ws_result.num_bytes_to], tcp_buffer)?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        },
        WebSocketReceiveMessageType::Binary => {
            // ignored
            Ok(true)
        },
        WebSocketReceiveMessageType::CloseCompleted => {
            Ok(false)
        },
        WebSocketReceiveMessageType::CloseMustReply => {
            let to_send = web_socket.write(WebSocketSendMessageType::CloseReply, true, &ws_buffer[..ws_result.num_bytes_to], tcp_buffer)?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        },
        WebSocketReceiveMessageType::Ping => {
            let to_send = web_socket.write(WebSocketSendMessageType::Pong, true, &ws_buffer[..ws_result.num_bytes_to], tcp_buffer)?;
            write_to_stream(stream, &tcp_buffer[..to_send])?;
            Ok(true)
        },
        WebSocketReceiveMessageType::Pong => {
            println!("Received Pong");
            Ok(true)
        }
    }
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:1337")?;

    // accept connections and process them serially
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(|| {
                    match handle_client(stream) {
                        Ok(()) => println!("Connection closed"),
                        Err(e) => println!("Error: {:?}", e)
                    }
                });
            },
            Err(e) => {
                println!("Failed to establish a connection: {}", e)
            }
        }
    }

    Ok(())
}