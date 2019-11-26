# embedded-websocket
A lightweight rust websocket library for embedded systems (no_std)

This library facilitates the encoding and decoding of websocket messages and can be used for both clients and servers. The library is intended to be used in constrained memory environments like embedded microcontrollers which cannot reference the rust standard library. The library will work with arbitrarily small buffers regardless of websocket frame size as long as the websocket header can be read (2 - 14 bytes depending)

### Running the examples

To run the demo web server

```
cargo run --example server
```
To run the demo websocket client
```
cargo run --example client
```
or use this url in your browser `http://127.0.0.1:1337/`

### Working example project

See https://github.com/ninjasource/led-display-websocket-demo for a complete end-to-end example of this library working with and stm32 m3 bluepill MCU and an embedded ethernet card.

### Example websocket client usage:
The following example initiates a opening handshake, checks the handshake response, sends a short message, initiates a close handshake, checks the close handshake response and quits.

```rust
use embedded_websocket as ws;

let mut buffer1: [u8; 1000] = [0; 1000];
let mut buffer2: [u8; 1000] = [0; 1000];
// use ws::random::EmptyRng::new() as an alternative below
let mut websocket = ws::WebSocket::new_client(rand::thread_rng());

// initiate a websocket opening handshake
let websocket_options = WebSocketOptions {
    path: "/chat",
    host: "localhost",
    origin: "http://localhost",
    sub_protocols: None,
    additional_headers: None,
};
let (_len, web_socket_key) = websocket.client_connect(&websocket_options, &mut buffer1)?;

// ... open TCP Stream and write len bytes from buffer1 to stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
let received_size = 0;

// check the server response against the websocket_key we generated
websocket.client_accept(&web_socket_key, &mut buffer1[..received_size])?;

// send a Text websocket frame
let _len = websocket.write(
    ws::WebSocketSendMessageType::Text,
    true,
    &"hello".as_bytes(),
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// the server (in this case) echos the text frame back. Read it. You can check the ws_result for frame type
let ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;
let _response = std::str::from_utf8(&buffer2[..ws_result.len_to])?;

// initiate a close handshake
let _len = websocket.close(
    ws::WebSocketCloseStatusCode::NormalClosure,
    None,
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// check the close handshake response from the server
let _ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;

// ... close handshake is complete, close the TCP connection

```

### Example websocket server usage:
The following example expects an http websocket upgrade message, sends a handshake response, reads a Text frame, echo's the frame back, reads a Close frame, sends a Close frame response.

```rust
use embedded_websocket as ws;

let mut buffer1: [u8; 1000] = [0; 1000];
let mut buffer2: [u8; 1000] = [0; 1000];
let mut websocket = ws::WebSocketServer::new_server();

// ... read some data from a TCP stream into buffer1 ...
let received_size = 0;

// repeat the read above and check below until Error::HttpHeaderIncomplete is no longer returned
// (i.e. \r\n\r\n has been read from the stream as per the http spec)
let header = ws::read_http_header(&buffer1[..received_size])?;
let ws_context = header.websocket_context.unwrap();

// check for Some(http_header.websocket_context)
let _len = websocket.server_accept(&ws_context.sec_websocket_key, None, &mut buffer1)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// in this particular example we get a Text message below which we simply want to echo back
let ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;

// text messages are always utf8 encoded strings
let _response = std::str::from_utf8(&buffer2[..ws_result.len_to])?; // log this

// echo text message back
let _len = websocket.write(
    ws::WebSocketSendMessageType::Text,
    true,
    &buffer2[..ws_result.len_to],
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// in this example say we get a CloseMustReply message below, we need to respond to complete the close handshake
let ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;
let _len = websocket.write(
    ws::WebSocketSendMessageType::CloseReply,
    true,
    &buffer2[..ws_result.len_to],
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream...
// ... close the TCP Stream ...
```

### License 

Licensed under either MIT or Apache-2.0 at your option
