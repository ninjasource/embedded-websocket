use super::*;
use core::borrow::{Borrow, BorrowMut};

pub struct DummyRng {}

impl DummyRng {
    pub fn new() -> DummyRng {
        DummyRng {}
    }
}

impl RngCore for DummyRng {
    fn next_u32(&mut self) -> u32 {
        0
    }
    fn next_u64(&mut self) -> u64 {
        0
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {}
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> core::result::Result<(), rand_core::Error> {
        Ok(())
    }
}

pub struct WebSocket<T>
where
    T: RngCore,
{
    is_client: bool,
    rng: T,
    continuation_frame_op_code: Option<WebSocketOpCode>,
    pub state: WebSocketState,
    continuation_read: Option<ContinuationRead>,
}

impl<T> WebSocket<T>
where
    T: RngCore,
{
    pub fn new(is_client: bool, rng: T) -> WebSocket<T> {
        WebSocket {
            is_client,
            rng,
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    pub fn client_connect(
        &mut self,
        websocket_options: &WebSocketOptions,
        to: &mut [u8],
    ) -> Result<(usize, WebSocketKey)> {
        let mut http_request: String<U1024> = String::new();
        let mut key_as_base64: [u8; 24] = [0; 24];

        let mut key: [u8; 16] = [0; 16];
        self.rng.fill_bytes(&mut key);
        base64::encode(&key, &mut key_as_base64);
        let sec_websocket_key: String<U24> = String::from(str::from_utf8(&key_as_base64)?);
        let port: String<U8> = String::try_from(websocket_options.port)?;

        http_request.push_str("GET ")?;
        http_request.push_str(websocket_options.path)?;
        http_request.push_str(" HTTP/1.1\r\nHost: ")?;
        http_request.push_str(websocket_options.host)?;
        http_request.push_str(":")?;
        http_request.push_str(port.as_str())?;
        http_request
            .push_str("\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ")?;
        http_request.push_str(sec_websocket_key.as_str())?;
        http_request.push_str("\r\nOrigin: http://")?;
        http_request.push_str(websocket_options.host)?;
        http_request.push_str(":")?;
        http_request.push_str(port.as_str())?;

        // turn sub protocol list into a CSV list
        http_request.push_str("\r\nSec-WebSocket-Protocol: ")?;
        if let Some(sub_protocols) = websocket_options.sub_protocols {
            for (i, sub_protocol) in sub_protocols.iter().enumerate() {
                http_request.push_str(sub_protocol)?;
                if i < (sub_protocols.len() - 1) {
                    http_request.push_str(", ")?;
                }
            }
        }
        http_request.push_str("\r\n")?;

        if let Some(additional_headers) = websocket_options.additional_headers {
            for additional_header in additional_headers.iter() {
                http_request.push_str(additional_header)?;
                http_request.push_str("\r\n")?;
            }
        }

        http_request.push_str("Sec-WebSocket-Version: 13\r\n\r\n")?;
        to[..http_request.len()].copy_from_slice(http_request.as_bytes());
        self.state = WebSocketState::Connecting;
        Ok((http_request.len(), sec_websocket_key))
    }

    pub fn client_accept(
        &mut self,
        sec_websocket_key: &WebSocketKey,
        from: &[u8],
    ) -> Result<Option<WebSocketSubProtocol>> {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut response = httparse::Response::new(&mut headers);
        if response.parse(&from)?.is_complete() {
            match response.code {
                Some(101) => {
                    // we are ok
                }
                _ => {
                    self.state = WebSocketState::Aborted;
                    return Err(Error::HttpResponseCodeInvalid);
                }
            };

            let mut sec_websocket_protocol: Option<WebSocketSubProtocol> = None;
            for item in response.headers.iter() {
                match item.name {
                    "Sec-WebSocket-Accept" => {
                        let mut output = [0; 28];
                        build_accept_string(&sec_websocket_key, &mut output)?;
                        let expected_accept_string = str::from_utf8(&output)?;
                        let actual_accept_string = str::from_utf8(item.value)?;
                        if actual_accept_string != expected_accept_string {
                            self.state = WebSocketState::Aborted;
                            return Err(Error::AcceptStringInvalid);
                        }
                    }
                    "Sec-WebSocket-Protocol" => {
                        sec_websocket_protocol = Some(String::from(str::from_utf8(item.value)?));
                    }
                    &_ => {
                        // ignore all other headers
                    }
                }
            }

            self.state = WebSocketState::Open;
            Ok(sec_websocket_protocol)
        } else {
            Err(Error::HttpHeaderIncomplete)
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

        self.write_frame(from, to, op_code, end_of_message)
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
                self.write_frame(&from_buffer, to, WebSocketOpCode::ConnectionClose, true)
            } else {
                let mut from_buffer: [u8; 2] = [0; 2];
                BigEndian::write_u16(&mut from_buffer, close_status as u16);
                self.write_frame(&from_buffer, to, WebSocketOpCode::ConnectionClose, true)
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

    fn write_frame(
        &mut self,
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
        const MASK_KEY_SIZE: usize = 4;
        let mut header_size = 0;
        let mask_bit_set_as_byte = if self.is_client { 0x80 } else { 0x00 };
        let payload_len = buf.len() + if self.is_client { MASK_KEY_SIZE } else { 0 };

        // write header followed by the payload
        // header size depends on how large the payload is
        if count < 126 {
            if payload_len + BYTE_HEADER_SIZE > to_buffer.len() {
                return Err(Error::WriteToBufferTooSmall);
            }
            to_buffer[0] = byte1;
            to_buffer[1] = mask_bit_set_as_byte | count as u8;
            header_size = BYTE_HEADER_SIZE;
        } else if count < 65535 {
            if payload_len + SHORT_HEADER_SIZE > to_buffer.len() {
                return Err(Error::WriteToBufferTooSmall);
            }
            to_buffer[0] = byte1;
            to_buffer[1] = mask_bit_set_as_byte | 126;
            LittleEndian::write_u16(&mut to_buffer[2..], count as u16);
            header_size = SHORT_HEADER_SIZE;
        } else {
            if payload_len + LONG_HEADER_SIZE > to_buffer.len() {
                return Err(Error::WriteToBufferTooSmall);
            }
            to_buffer[0] = byte1;
            to_buffer[1] = mask_bit_set_as_byte | 127;
            LittleEndian::write_u64(&mut to_buffer[2..], count as u64);
            header_size = LONG_HEADER_SIZE;
        }

        // sent by client - need to mask the data
        // we need to mask the bytes to prevent web server caching
        if self.is_client {
            let mut mask_key = [0; MASK_KEY_SIZE];
            self.rng.fill_bytes(&mut mask_key); // clients always have an rng instance
            to_buffer[header_size..header_size + MASK_KEY_SIZE].copy_from_slice(&mask_key);
            let to_buffer_start = header_size + MASK_KEY_SIZE;

            let mut i = 0;
            for (from, to) in from_buffer[..count]
                .iter()
                .zip(&mut to_buffer[to_buffer_start..to_buffer_start + count])
            {
                *to = *from ^ mask_key[i % MASK_KEY_SIZE];
                i = i + 1;
            }

            Ok(to_buffer_start + count)
        } else {
            to_buffer[header_size..header_size + count].copy_from_slice(&from_buffer[..count]);
            Ok(header_size + count)
        }
    }
}
