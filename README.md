# embedded-websocket
A lightweight rust websocket library for embedded systems (no_std)

This library facilitates the encoding and decoding of websocket messages and can be used for both clients and servers. The library is intended to be used in constrained memory environments like embedded microcontrollers which cannot reference the rust standard library. The library will work with arbitrarily small buffers regardless of websocket frame size as long as the websocket header can be read (2 - 14 bytes depending)

### No_std support

You can use this library without linking the rust standard library. In your Cargo.toml file make sure you set the default features to false. You will not be able to use modules that require the standard library like `framer`. For example:
```
embedded-websocket = { version = "x.x.x", default-features = false }
```

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
    use std::net::TcpStream;

    // open a TCP stream to localhost port 1337
    let address = "127.0.0.1:1337";
    println!("Connecting to: {}", address);
    let mut stream = TcpStream::connect(address)?;
    println!("Connected.");

    let mut read_buf: [u8; 4000] = [0; 4000];
    let mut write_buf: [u8; 4000] = [0; 4000];
    let mut frame_buf: [u8; 4000] = [0; 4000];
    let mut ws_client = ws::WebSocketClient::new_client(rand::thread_rng());

    // initiate a websocket opening handshake
    let websocket_options = ws::WebSocketOptions {
        path: "/chat",
        host: "localhost",
        origin: "http://localhost:1337",
        sub_protocols: None,
        additional_headers: None,
    };

    let mut websocket = ws::framer::Framer::new(&mut read_buf, &mut write_buf, &mut ws_client, &mut stream);
    websocket.connect(&websocket_options)?;

    let message = "Hello, World!";
    websocket.write(ws::WebSocketSendMessageType::Text, true, message.as_bytes())?;

    while let Some(s) = websocket.read_text(&mut frame_buf)? {
        println!("Received: {}", s);

        // close the websocket after receiving the first message
        websocket.close(ws::WebSocketCloseStatusCode::NormalClosure, None)?;
        println!("Sent close handshake");
    }

    while let Some(s) = websocket.read_text(&mut frame_buf)? {
        println!("Received: {}", s);
    }

    println!("Connection closed");    
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
let len = websocket.server_accept(&ws_context.sec_websocket_key, None, &mut buffer1)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// in this particular example we get a Text message below which we simply want to echo back
let ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;

// text messages are always utf8 encoded strings
let response = std::str::from_utf8(&buffer2[..ws_result.len_to])?; // log this

// echo text message back
let len = websocket.write(
    ws::WebSocketSendMessageType::Text,
    true,
    &buffer2[..ws_result.len_to],
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...

// in this example say we get a CloseMustReply message below, we need to respond to complete the close handshake
let ws_result = websocket.read(&buffer1[..received_size], &mut buffer2)?;
let len = websocket.write(
    ws::WebSocketSendMessageType::CloseReply,
    true,
    &buffer2[..ws_result.len_to],
    &mut buffer1,
)?;

// ... write len bytes from buffer1 to TCP Stream...
// ... close the TCP Stream ...
```

### Upgrading from version "0.1.1" to "0.2.0"
Be sure to update your Cargo.toml file to reference the following:
```
rand_core = "0.5"
```
and rand if you use it:
```
rand = "0.7"
```
Run Cargo update

### Upgrading from version "0.2.0" to "0.3.0"
Use ```ws::WebSocketClient::new_client(rand::thread_rng())``` to create a client and ```ws::WebSocketServer::new_server()``` to create a server
Use ```WebSocketClient<EmptyRng>``` instead of ```WebSocket<EmptyRng>```

### License 

Licensed under either MIT or Apache-2.0 at your option
