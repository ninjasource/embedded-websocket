use super::*;

pub struct EmptyRng {}

impl EmptyRng {
    pub fn new() -> EmptyRng {
        EmptyRng {}
    }
}

impl RngCore for EmptyRng {
    fn next_u32(&mut self) -> u32 {
        0
    }
    fn next_u64(&mut self) -> u64 {
        0
    }
    fn fill_bytes(&mut self, _dest: &mut [u8]) {}
    fn try_fill_bytes(&mut self, _dest: &mut [u8]) -> core::result::Result<(), rand_core::Error> {
        Ok(())
    }
}

pub type WebSocketServer = WebSocket<EmptyRng>;

impl WebSocketServer {
    pub fn new_server() -> WebSocketServer {
        let rng = EmptyRng::new();
        WebSocket {
            is_client: false,
            rng,
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    pub fn server_accept(
        &mut self,
        sec_websocket_key: &WebSocketKey,
        sec_websocket_protocol: Option<&WebSocketSubProtocol>,
        to: &mut [u8],
    ) -> Result<usize> {
        self.state = WebSocketState::Connecting;

        let mut http_response: String<U1024> = String::new();
        http_response.push_str(
            "HTTP/1.1 101 Switching Protocols\r\n\
             Connection: Upgrade\r\nUpgrade: websocket\r\n",
        )?;

        // if the user has specified a sub protocol
        if let Some(sec_websocket_protocol) = sec_websocket_protocol {
            http_response.push_str("Sec-WebSocket-Protocol: ")?;
            http_response.push_str(sec_websocket_protocol)?;
            http_response.push_str("\r\n")?;
        }

        let mut output = [0; 28];
        build_accept_string(sec_websocket_key, &mut output)?;
        let accept_string = str::from_utf8(&output)?;
        http_response.push_str("Sec-WebSocket-Accept: ")?;
        http_response.push_str(accept_string)?;
        http_response.push_str("\r\n\r\n")?;

        // save the response to the buffer
        to[..http_response.len()].copy_from_slice(http_response.as_bytes());
        self.state = WebSocketState::Open;

        Ok(http_response.len())
    }
}
