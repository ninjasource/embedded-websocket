use super::*;

pub struct WebSocketServer {
    continuation_frame_op_code: Option<WebSocketOpCode>,
    state: WebSocketState,
    continuation_read: Option<ContinuationRead>,
}

impl WebSocketServer {
    pub fn new() -> WebSocketServer {
        WebSocketServer {
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    pub fn accept(
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

    #[inline]
    pub fn get_state(&self) -> WebSocketState {
        self.state
    }

    #[cfg(test)]
    pub fn set_state(&mut self, state: WebSocketState) {
        self.state = state;
    }

    pub fn read(&mut self, from: &[u8], to: &mut [u8]) -> Result<WebSocketReadResult> {
        let frame = self.read_frame(from, to)?;
        let read_result = frame.to_readresult(&self.continuation_frame_op_code, &self.state)?;
        if frame.op_code == WebSocketOpCode::ConnectionClose
            && self.state == WebSocketState::CloseSent
        {
            self.state = WebSocketState::Closed;
        }

        Ok(read_result)
    }

    pub fn write(
        &mut self,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        from: &[u8],
        to: &mut [u8],
    ) -> Result<usize> {
        let op_code = message_type.to_op_code();
        if op_code == WebSocketOpCode::ConnectionClose {
            self.state = WebSocketState::Closed
        }

        write_frame(from, to, op_code, end_of_message)
    }

    pub fn close(
        &mut self,
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
        to: &mut [u8],
    ) -> Result<(usize)> {
        if self.state == WebSocketState::Open {
            self.state = WebSocketState::CloseSent;
            if let Some(status_description) = status_description {
                let mut from_buffer: Vec<u8, U256> = Vec::new();
                BigEndian::write_u16(&mut from_buffer, close_status as u16);

                // restrict the max size of the status_description
                let len = if status_description.len() < 254 {
                    status_description.len()
                } else {
                    254
                };

                from_buffer.extend(status_description[..len].as_bytes());

                write_frame(&from_buffer, to, WebSocketOpCode::ConnectionClose, true)
            } else {
                let mut from_buffer: [u8; 2] = [0; 2];
                BigEndian::write_u16(&mut from_buffer, close_status as u16);
                write_frame(&from_buffer, to, WebSocketOpCode::ConnectionClose, true)
            }
        } else {
            Err(Error::WebSocketNotOpen)
        }
    }

    fn read_frame(&mut self, from_buffer: &[u8], to_buffer: &mut [u8]) -> Result<WebSocketFrame> {
        match &mut self.continuation_read {
            Some(continuation_read) => {
                let result = read_continuation(continuation_read, from_buffer, to_buffer)?;
                if result.is_fin_bit_set {
                    self.continuation_read = None;
                }
                Ok(result)
            }
            None => {
                let (result, continuation_read) = read_frame(from_buffer, to_buffer)?;
                self.continuation_frame_op_code = if result.is_fin_bit_set {
                    None
                } else {
                    Some(result.op_code)
                };

                self.continuation_read = continuation_read;
                Ok(result)
            }
        }
    }
}

fn write_frame(
    from_buffer: &[u8],
    to_buffer: &mut [u8],
    op_code: WebSocketOpCode,
    end_of_message: bool,
) -> Result<usize> {
    // we are working with byte array of 64 elements so all the unwraps below are ok
    let mut buf: Vec<u8, U64> = Vec::new();

    let fin_bit_set_as_byte: u8 = if end_of_message { 0x80 } else { 0x00 };
    let byte1: u8 = fin_bit_set_as_byte | op_code as u8;
    let count = from_buffer.len();
    const BYTE_HEADER_SIZE: usize = 2;
    const SHORT_HEADER_SIZE: usize = 4;
    const LONG_HEADER_SIZE: usize = 10;

    // write header followed by the payload
    // header size depends on how large the payload is
    if count < 126 {
        if buf.len() + BYTE_HEADER_SIZE > to_buffer.len() {
            return Err(Error::WriteToBufferTooSmall);
        }
        let byte2 = count as u8;
        to_buffer[0] = byte1;
        to_buffer[1] = byte2;
        to_buffer[BYTE_HEADER_SIZE..BYTE_HEADER_SIZE + count].copy_from_slice(&from_buffer);
        Ok(BYTE_HEADER_SIZE + count)
    } else if count < 65535 {
        if buf.len() + SHORT_HEADER_SIZE > to_buffer.len() {
            return Err(Error::WriteToBufferTooSmall);
        }
        let byte2 = 126;
        to_buffer[0] = byte1;
        to_buffer[1] = byte2;
        LittleEndian::write_u16(&mut to_buffer[2..], count as u16);
        to_buffer[SHORT_HEADER_SIZE..SHORT_HEADER_SIZE + count].copy_from_slice(&from_buffer);
        Ok(SHORT_HEADER_SIZE + count)
    } else {
        if buf.len() + LONG_HEADER_SIZE > to_buffer.len() {
            return Err(Error::WriteToBufferTooSmall);
        }

        let byte2 = 127;
        to_buffer[0] = byte1;
        to_buffer[1] = byte2;
        LittleEndian::write_u64(&mut to_buffer[2..], count as u64);
        to_buffer[LONG_HEADER_SIZE..LONG_HEADER_SIZE + count].copy_from_slice(&from_buffer);
        Ok(LONG_HEADER_SIZE + count)
    }
}
