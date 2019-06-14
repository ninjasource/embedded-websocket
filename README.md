# embedded-websockets
A lightweight rust websocket library for embedded systems (no_std)

This library facilitates the encoding and decoding of websocket messages and can be used for both clients and servers. The library is intended to be used in constrained memory environments like embedded microcontrollers which cannot reference the rust standard library. The library will work with arbitrarily small buffers regardless of websocket frame size as long as the websocket header can be read (2 - 14 bytes depending)

### Example websocket client usage:
The following example initiates a opening handshake, checks the handshake response, sends a short message, initiates a close handshake, checks the close handshake response and quits.

```rust

let mut buffer1: [u8; 1000] = [0; 1000];
let mut buffer2: [u8; 1000] = [0; 1000];
let mut ws_client = WebSocketClient::new_client(rand::thread_rng());

// initiate a websocket opening handshake
let websocket_options = WebSocketOptions {
    path: "/chat",
    host: "localhost",
    port: 1337,
    sub_protocols: None,
    additional_headers: None,
};
let (len, web_socket_key) = ws_client.connect(&websocket_options, &mut buffer1)?;

 ... open TCP Stream and write len bytes from buffer1 to stream ...
 ... read some received_size data from a TCP stream into buffer1 ...

// check the server response against the websocket_key we generated
ws_client.client_accept(&web_socket_key, &mut buffer1[..received_size])?;

// send a Text websocket frame
let len = ws_client.write(WebSocketSendMessageType::Text, true, &"hello".as_bytes(), &mut buffer1)?;

 ... write len bytes from buffer1 to TCP Stream ...
 ... read some received_size data from a TCP stream into buffer1 ...

// the server (in this case) echos the text frame back. Read it. You can check the ws_result for frame type
let ws_result = ws_client.read(&buffer1[..received_size], &mut buffer2)?;
let response = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?;

// initiate a close handshake
let len = ws_client.close(WebSocketCloseStatusCode::NormalClosure, None, &mut buffer1)?;

  ... write len bytes from buffer1 to TCP Stream ...
  ... read some received_size data from a TCP stream into buffer1 ...

// check the close handshake response from the server
let ws_result = ws_client.read(&buffer1[..received_size], &mut ws_buffer2)?;

 ... close handshake is complete, close the TCP connection
```

### Example websocket server usage:
The following example expects an http websocket upgrade message, sends a handshake response, reads a Text frame, echo's the frame back, reads a Close frame, sends a Close frame response.

```rust

let mut buffer1: [u8; 1000] = [0; 1000];
let mut buffer2: [u8; 1000] = [0; 1000];
let mut web_socket = WebSocketServer::new_server();

 ... read some data from a TCP stream into buffer1 ...

// repeat the read above and check below until Error::HttpHeaderIncomplete is no longer returned 
// (i.e. \r\n\r\n has been read from the stream as per the http spec) 
let http_header = websockets::read_http_header(&buffer1[..received_size])?;

// check for Some(http_header.websocket_context)
let len = web_socket.server_accept(&websocket_context.sec_websocket_key, None, buffer1)?;

 ... write len bytes from buffer1 to TCP Stream ...
 ... read some received_size data from a TCP stream into buffer1 ...

// in this particular example we get a Text message below which we simply want to echo back
let ws_result = web_socket.read(&buffer1[..received_size], &mut buffer2)?;

// text messages are always utf8 encoded strings
let response = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?; // log this

// echo text message back
let len = web_socket.write(WebSocketSendMessageType::Text, true, &buffer2[..ws_result.num_bytes_to], buffer1)?;

 ... write len bytes from buffer1 to TCP Stream ...
 ... read some received_size data from a TCP stream into buffer1 ...

// in this example say we get a CloseMustReply message below, we need to respond to complete the close handshake 
let ws_result = web_socket.read(&buffer1[..received_size], &mut buffer2)?;
let len = web_socket.write(WebSocketSendMessageType::CloseReply, true, &buffer2[..ws_result.num_bytes_to], 
  buffer1)?;

 ... write len bytes from buffer1 to TCP Stream...
 ... close the TCP Stream ...
```
