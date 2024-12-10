use core::ops::Deref;
use futures::{Sink, SinkExt, Stream, StreamExt};
use std::io;

use bytes::{BufMut, BytesMut};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Decoder, Encoder, Framed};

use embedded_websocket::{
    framer_async::{Framer, FramerError, ReadResult},
    read_http_header, WebSocketContext, WebSocketSendMessageType, WebSocketServer,
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
async fn main() -> Result<(), FramerError<io::Error>> {
    let addr = "127.0.0.1:1337";
    let listener = TcpListener::bind(addr).await.map_err(FramerError::Io)?;
    println!("Listening on: {}", addr);

    // accept connections and process them in parallel
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(async move {
                    match handle_client(stream).await {
                        Ok(()) => println!("Connection closed"),
                        Err(e) => println!("Error: {:?}", e),
                    };
                });
            }
            Err(e) => println!("Failed to establish a connection: {}", e),
        }
    }
}

async fn handle_client(stream: TcpStream) -> Result<(), FramerError<io::Error>> {
    println!(
        "Client connected {}",
        stream.peer_addr().map_err(FramerError::Io)?
    );

    let mut buffer = [0u8; 4000];
    let codec = MyCodec::new();
    let mut stream = Framed::new(stream, codec);

    if let Some((websocket_context, rx_remainder_len)) =
        read_header(&mut stream, &mut buffer).await?
    {
        // this is a websocket upgrade HTTP request
        let websocket = WebSocketServer::new_server();
        let mut framer = Framer::new_with_rx(websocket, rx_remainder_len);

        // complete the opening handshake with the client
        framer
            .accept(&mut stream, &mut buffer, &websocket_context)
            .await?;
        println!("Websocket connection opened");

        // read websocket frames
        while let Some(read_result) = framer.read(&mut stream, &mut buffer).await {
            if let ReadResult::Text(text) = read_result? {
                println!("Received: {}", text);

                // copy text to satisfy borrow checker
                let text = Vec::from(text.as_bytes());

                // send the text back to the client
                framer
                    .write(
                        &mut stream,
                        &mut buffer,
                        WebSocketSendMessageType::Text,
                        true,
                        &text,
                    )
                    .await?
            }
        }

        println!("Closing websocket connection");

        Ok(())
    } else {
        Ok(())
    }
}

async fn read_header<'a, B: Deref<Target = [u8]>, E>(
    stream: &mut (impl Stream<Item = Result<B, E>> + Sink<&'a [u8], Error = E> + Unpin),
    buffer: &'a mut [u8],
) -> Result<Option<(WebSocketContext, usize)>, FramerError<E>> {
    let mut read_cursor = 0usize;

    loop {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut request = httparse::Request::new(&mut headers);

        match stream.next().await {
            Some(Ok(input)) => {
                if buffer.len() < read_cursor + input.len() {
                    return Err(FramerError::RxBufferTooSmall(read_cursor + input.len()));
                }

                // copy to start of buffer (unlike Framer::read())
                buffer[read_cursor..read_cursor + input.len()].copy_from_slice(&input);
                read_cursor += input.len();

                if let httparse::Status::Complete(len) = request
                    .parse(&buffer[0..read_cursor])
                    .map_err(FramerError::HttpHeader)?
                {
                    // if we read exactly the right amount of bytes for the HTTP header then read_cursor would be 0
                    let headers = request.headers.iter().map(|f| (f.name, f.value));
                    match read_http_header(headers).map_err(FramerError::WebSocket)? {
                        Some(websocket_context) => match request.path {
                            Some("/chat") => {
                                let remaining_len = read_cursor - len;
                                for i in 0..remaining_len {
                                    buffer[buffer.len() - remaining_len + i] = buffer[len + i]
                                }
                                return Ok(Some((websocket_context, remaining_len)));
                            }
                            _ => return_404_not_found(stream, request.path).await?,
                        },
                        None => {
                            handle_non_websocket_http_request(stream, request.path).await?;
                        }
                    }
                    return Ok(None);
                }
            }
            Some(Err(e)) => {
                return Err(FramerError::Io(e));
            }
            None => return Ok(None),
        }
    }
}

async fn handle_non_websocket_http_request<'a, B, E>(
    stream: &mut (impl Stream<Item = Result<B, E>> + Sink<&'a [u8], Error = E> + Unpin),
    path: Option<&str>,
) -> Result<(), FramerError<E>> {
    println!("Received file request: {:?}", path);

    match path {
        Some("/") => {
            stream
                .send(ROOT_HTML.as_bytes())
                .await
                .map_err(FramerError::Io)?;
            stream.flush().await.map_err(FramerError::Io)?;
        }
        unknown_path => {
            return_404_not_found(stream, unknown_path).await?;
        }
    };

    Ok(())
}

async fn return_404_not_found<'a, B, E>(
    stream: &mut (impl Stream<Item = Result<B, E>> + Sink<&'a [u8], Error = E> + Unpin),
    unknown_path: Option<&str>,
) -> Result<(), FramerError<E>> {
    println!("Unknown path: {:?}", unknown_path);
    let html = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    stream
        .send(html.as_bytes())
        .await
        .map_err(FramerError::Io)?;
    stream.flush().await.map_err(FramerError::Io)?;
    Ok(())
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
