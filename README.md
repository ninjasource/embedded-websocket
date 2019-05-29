# embedded-websockets
A rust websocket library for embedded systems (no_std)

This library facilitates the encoding and decoding of websocket messages and can be used for both clients and servers. The library is intended to be used in constrained memory environments like embedded microcontrollers which cannot reference the rust standard library. The library will work with arbitrarily small buffers regardless of websocket frame size as long as the websocket header can be read (2 - 14 bytes depending)

### Example websocket client usage:
The following example initiates a opening handshake, checks the handshake response, sends a short message, initiates a close handshake, checks the close handshake response and quits.

```rust
let (len, web_socket_key) = ws_client.client_initiate_opening_handshake("/chat", "localhost", "1337", None, None, &mut buffer1)?;
// ... open TCP Stream and write len bytes from buffer1 to stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
ws_client.client_complete_opening_handshake(web_socket_key.as_str(), &mut buffer1[..received_size])?;
let len = ws_client.write(WebSocketSendMessageType::Text, true, &"hello".as_bytes(), &mut buffer1)?;
// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
// below should be a Text message type 
let ws_result = ws_client.read(&buffer1[..received_size], &mut buffer2)?;
let response = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?;
let len = ws_client.close(WebSocketCloseStatusCode::NormalClosure, None, &mut buffer1)?;
// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
// below should be a Close message type 
let ws_result = ws_client.read(&buffer1[..received_size], &mut ws_buffer2)?;
// ... close handshake is complete, close the TCP connection
```

### Example websocket server usage:
The following example expects an http websocket upgrade message, sends a handshake response, reads a Text frame, echo's the frame back, reads a Close frame, sends a Close frame response.

```rust
// ... read some data from a TCP stream into buffer1 ...
// repeat below until Error::HttpHeaderIncomplete is no longer returned (i.e. \r\n\r\n) ***
let http_header = websockets::read_http_header(&buffer1[..received_size])?;
// check for Some(http_header.websocket_context)
let len = web_socket.server_respond_to_opening_handshake(&websocket_context.sec_websocket_key, None, &mut buffer1)?;
// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
// in this example say we get a Text message below which we simply want to echo back
let ws_result = web_socket.read(&buffer1[..received_size], &mut buffer2)?;
let response = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to])?; // log this
let len = web_socket.write(WebSocketSendMessageType::Text, true, &buffer2[..ws_result.num_bytes_to], buffer1)?;
// ... write len bytes from buffer1 to TCP Stream ...
// ... read some received_size data from a TCP stream into buffer1 ...
// in this example say we get a CloseMustReply message below ***
let ws_result = web_socket.read(&buffer1[..received_size], &mut buffer2)?;
let len = web_socket.write(WebSocketSendMessageType::CloseReply, true, &buffer2[..ws_result.num_bytes_to], buffer1)?;
// ... write len bytes from buffer1 to TCP Stream...
// ... close the TCP Stream ...
```
