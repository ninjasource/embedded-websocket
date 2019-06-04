// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// This library facilitates the encoding and decoding of websocket messages and can be used
// for both clients and servers. The library is intended to be used in constrained memory
// environments like embedded microcontrollers which cannot reference the rust standard library.
// The library will work with arbitrarily small buffers regardless of websocket frame size
// as long as the websocket header can be read (2 - 14 bytes depending)

#![no_std]

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use core::{result, str};
use heapless::consts::*; // these are for constants like U4, U16, U24, U32
use heapless::{String, Vec};
use sha1::Sha1;

// the world's worst random number generator - but hey, it's just for http caching
struct Random {
    seed: u8,
}

impl Random {
    fn new(seed: u8) -> Random {
        Random { seed }
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for i in 0..dest.len() {
            self.seed = if self.seed == 255 { 0 } else { self.seed + 1 };
            dest[i] = self.seed;
        }
    }
}

pub struct HttpHeader {
    pub path: String<U128>,
    pub websocket_context: Option<WebSocketContext>,
}

pub struct WebSocketContext {
    // Max 3 sub protocols of size 24 bytes each
    pub sec_websocket_protocol_list: Vec<String<U24>, U3>,
    pub sec_websocket_key: String<U24>,
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum WebSocketSendMessageType {
    Text = 1,
    Binary = 2,
    Ping = 9,
    Pong = 10,
    CloseReply = 11,
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum WebSocketReceiveMessageType {
    Text = 1,
    Binary = 2,
    CloseCompleted = 7,
    CloseMustReply = 8,
    Ping = 9,
    Pong = 10,
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum WebSocketCloseStatusCode {
    NormalClosure = 1000,
    EndpointUnavailable = 1001,
    ProtocolError = 1002,
    InvalidMessageType = 1003,
    Empty = 1005,
    InvalidPayloadData = 1007,
    PolicyViolation = 1008,
    MessageTooBig = 1009,
    MandatoryExtension = 1010,
    InternalServerError = 1011,
}

#[derive(Debug)]
pub struct WebSocketReadResult {
    pub num_bytes_from: usize,
    pub num_bytes_to: usize,
    pub end_of_message: bool,
    pub close_status: Option<WebSocketCloseStatusCode>,
    pub message_type: WebSocketReceiveMessageType,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum WebSocketOpCode {
    ContinuationFrame = 0,
    TextFrame = 1,
    BinaryFrame = 2,
    ConnectionClose = 8,
    Ping = 9,
    Pong = 10,
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum WebSocketState {
    None = 0,
    Connecting = 1,
    Open = 2,
    CloseSent = 3,
    CloseReceived = 4,
    Closed = 5,
    Aborted = 6,
}

const MASK_KEY_LEN: usize = 4;
pub type Result<T> = result::Result<T, Error>;

#[derive(PartialEq, Debug)]
pub enum Error {
    InvalidOpCode,
    InvalidContinuationFrame, // Not used
    InvalidFrameLength,       // Not used
    InvalidCloseStatusCode,
    WebSocketNotOpen,
    Utf8Error,
    Unknown,
    HttpHeader(httparse::Error),
    HttpHeaderNoPath,
    HttpHeaderIncomplete,
    WriteToBufferTooSmall,
    ReadFromBufferTooSmall,
    HttpResponseCodeInvalid,
    AcceptStringInvalid,
}

impl From<httparse::Error> for Error {
    fn from(err: httparse::Error) -> Error {
        Error::HttpHeader(err)
    }
}

impl From<str::Utf8Error> for Error {
    fn from(_: str::Utf8Error) -> Error {
        Error::Utf8Error
    }
}

impl From<()> for Error {
    fn from(_: ()) -> Error {
        Error::Unknown
    }
}

impl WebSocketOpCode {
    fn to_message_type(&self) -> Result<WebSocketReceiveMessageType> {
        match self {
            WebSocketOpCode::TextFrame => Ok(WebSocketReceiveMessageType::Text),
            WebSocketOpCode::BinaryFrame => Ok(WebSocketReceiveMessageType::Binary),
            _ => Err(Error::InvalidOpCode),
        }
    }
}

struct WebSocketFrame {
    is_fin_bit_set: bool,
    op_code: WebSocketOpCode,
    num_bytes_to: usize,
    num_bytes_from: usize,
    close_status: Option<WebSocketCloseStatusCode>,
}

pub struct WebSocket {
    is_client: bool,
    rng: Random,
    continuation_frame_op_code: Option<WebSocketOpCode>,
    state: WebSocketState,
    continuation_read: Option<ContinuationRead>,
}

// this is used when we cannot fit the entire websocket frame into the supplied buffer
pub struct ContinuationRead {
    op_code: WebSocketOpCode,
    count: usize,
    is_fin_bit_set: bool,
    mask_key: Option<[u8; 4]>,
}

pub fn read_http_header(buffer: &[u8]) -> Result<HttpHeader> {
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    if req.parse(&buffer)?.is_complete() {
        let path = match req.path {
            Some(path) => String::from(path),
            None => return Err(Error::HttpHeaderNoPath),
        };
        let mut sec_websocket_protocol_list: Vec<String<U24>, U3> = Vec::new();
        let mut is_websocket_request = false;
        let mut sec_websocket_key = String::new();

        for item in req.headers.iter() {
            match item.name {
                "Upgrade" => is_websocket_request = str::from_utf8(item.value)? == "websocket",
                "Sec-WebSocket-Protocol" => {
                    // extract a csv list of supported sub protocols
                    for item in str::from_utf8(item.value)?.split(',') {
                        if sec_websocket_protocol_list.len()
                            < sec_websocket_protocol_list.capacity()
                        {
                            // it is safe to unwrap here because we have checked
                            // the size of the list beforehand
                            sec_websocket_protocol_list
                                .push(String::from(item))
                                .unwrap();
                        }
                    }
                }
                "Sec-WebSocket-Key" => {
                    sec_websocket_key = String::from(str::from_utf8(item.value)?);
                }
                &_ => {
                    // ignore all other headers
                }
            }
        }

        let websocket_context = {
            if is_websocket_request {
                Some(WebSocketContext {
                    sec_websocket_protocol_list,
                    sec_websocket_key,
                })
            } else {
                None
            }
        };

        let header = HttpHeader {
            path,
            websocket_context,
        };

        return Ok(header);
    } else {
        return Err(Error::HttpHeaderIncomplete);
    }
}

impl WebSocket {
    pub fn new_server() -> WebSocket {
        WebSocket {
            is_client: false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    pub fn new_client() -> WebSocket {
        WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    pub fn client_initiate_opening_handshake(
        &mut self,
        uri: &str,
        host: &str,
        port: &str,
        sub_protocol: Option<&str>,
        additional_headers: Option<&str>,
        to_buffer: &mut [u8],
    ) -> Result<(usize, String<U24>)> {
        let mut key: [u8; 16] = [0; 16];
        self.rng.fill_bytes(&mut key);
        let mut key_as_base64: [u8; 24] = [0; 24];
        base64_encode(&key, &mut key_as_base64);
        let mut http_request: String<U1024> = String::new();
        let sec_websocket_key: String<U24> = String::from(str::from_utf8(&key_as_base64)?);
        http_request.push_str("GET ")?;
        http_request.push_str(uri)?;
        http_request.push_str(" HTTP/1.1\r\nHost: ")?;
        http_request.push_str(host)?;
        http_request.push_str(":")?;
        http_request.push_str(port)?;
        http_request
            .push_str("\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ")?;
        http_request.push_str(sec_websocket_key.as_str())?;
        http_request.push_str("\r\nOrigin: http://")?;
        http_request.push_str(host)?;
        http_request.push_str(":")?;
        http_request.push_str(port)?;
        http_request.push_str("\r\nSec-WebSocket-Protocol: ")?;
        if let Some(sub_protocol) = sub_protocol {
            http_request.push_str(sub_protocol)?;
        }

        http_request.push_str("\r\n")?;
        if let Some(additional_headers) = additional_headers {
            http_request.push_str(additional_headers)?;
        }

        http_request.push_str("Sec-WebSocket-Version: 13\r\n\r\n")?;
        to_buffer[..http_request.len()].copy_from_slice(http_request.as_bytes());
        self.state = WebSocketState::Connecting;
        Ok((http_request.len(), sec_websocket_key))
    }

    pub fn client_complete_opening_handshake(
        &mut self,
        sec_websocket_key: &str,
        from_buffer: &[u8],
    ) -> Result<Option<String<U24>>> {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut response = httparse::Response::new(&mut headers);
        let mut sec_websocket_protocol: Option<String<U24>> = None;
        if response.parse(&from_buffer)?.is_complete() {
            match response.code {
                Some(101) => {
                    // we are ok
                }
                _ => {
                    self.state = WebSocketState::Aborted;
                    return Err(Error::HttpResponseCodeInvalid);
                }
            };

            for item in response.headers.iter() {
                match item.name {
                    "Sec-WebSocket-Accept" => {
                        let mut output = [0; 28];
                        build_accept_string(sec_websocket_key, &mut output)?;
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
        }

        self.state = WebSocketState::Open;
        Ok(sec_websocket_protocol)
    }

    pub fn server_respond_to_opening_handshake(
        &mut self,
        sec_websocket_key: &str,
        sec_websocket_protocol: Option<&str>,
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

    pub fn get_state(&self) -> WebSocketState {
        self.state
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
                let mut vec: Vec<u8, U256> = Vec::new();
                BigEndian::write_u16(&mut vec, close_status as u16);

                // restrict the max size of the status_description
                let len = if status_description.len() < 254 {
                    status_description.len()
                } else {
                    254
                };
                let len = if len < (to.len() - 8) {
                    len
                } else {
                    to.len() - 8
                };
                vec.extend(status_description[..len].as_bytes());

                return write_frame(
                    self.is_client,
                    &vec,
                    to,
                    WebSocketOpCode::ConnectionClose,
                    true,
                    &mut self.rng,
                );
            } else {
                BigEndian::write_u16(&mut to[..2], close_status as u16);
                return write_frame(
                    self.is_client,
                    &[],
                    to,
                    WebSocketOpCode::ConnectionClose,
                    true,
                    &mut self.rng,
                );
            }
        }

        Err(Error::WebSocketNotOpen)
    }

    pub fn write(
        &mut self,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        from: &[u8],
        to: &mut [u8],
    ) -> Result<usize> {
        let op_code = match message_type {
            WebSocketSendMessageType::Text => WebSocketOpCode::TextFrame,
            WebSocketSendMessageType::Binary => WebSocketOpCode::BinaryFrame,
            WebSocketSendMessageType::Ping => WebSocketOpCode::Ping,
            WebSocketSendMessageType::Pong => WebSocketOpCode::Pong,
            WebSocketSendMessageType::CloseReply => {
                self.state = WebSocketState::Closed;
                WebSocketOpCode::ConnectionClose
            }
        };

        write_frame(
            self.is_client,
            from,
            to,
            op_code,
            end_of_message,
            &mut self.rng,
        )
    }

    pub fn read(&mut self, from: &[u8], to: &mut [u8]) -> Result<WebSocketReadResult> {
        let frame = self.read_frame(from, to)?;

        match frame.op_code {
            WebSocketOpCode::Ping => Ok(WebSocketReadResult {
                num_bytes_from: frame.num_bytes_from,
                num_bytes_to: frame.num_bytes_to,
                end_of_message: frame.is_fin_bit_set,
                close_status: None,
                message_type: WebSocketReceiveMessageType::Ping,
            }),
            WebSocketOpCode::Pong => Ok(WebSocketReadResult {
                num_bytes_from: frame.num_bytes_from,
                num_bytes_to: frame.num_bytes_to,
                end_of_message: frame.is_fin_bit_set,
                close_status: None,
                message_type: WebSocketReceiveMessageType::Pong,
            }),
            WebSocketOpCode::ConnectionClose => {
                let message_type = if self.state == WebSocketState::CloseSent {
                    self.state = WebSocketState::Closed;
                    WebSocketReceiveMessageType::CloseCompleted
                } else {
                    WebSocketReceiveMessageType::CloseMustReply
                };

                Ok(WebSocketReadResult {
                    num_bytes_from: frame.num_bytes_from,
                    num_bytes_to: frame.num_bytes_to,
                    end_of_message: frame.is_fin_bit_set,
                    close_status: frame.close_status,
                    message_type,
                })
            }
            WebSocketOpCode::TextFrame => Ok(WebSocketReadResult {
                num_bytes_from: frame.num_bytes_from,
                num_bytes_to: frame.num_bytes_to,
                end_of_message: frame.is_fin_bit_set,
                close_status: None,
                message_type: WebSocketReceiveMessageType::Text,
            }),
            WebSocketOpCode::BinaryFrame => Ok(WebSocketReadResult {
                num_bytes_from: frame.num_bytes_from,
                num_bytes_to: frame.num_bytes_to,
                end_of_message: frame.is_fin_bit_set,
                close_status: None,
                message_type: WebSocketReceiveMessageType::Binary,
            }),
            WebSocketOpCode::ContinuationFrame => match self.continuation_frame_op_code {
                Some(cf_op_code) => Ok(WebSocketReadResult {
                    num_bytes_from: frame.num_bytes_from,
                    num_bytes_to: frame.num_bytes_to,
                    end_of_message: frame.is_fin_bit_set,
                    close_status: None,
                    message_type: cf_op_code.to_message_type()?,
                }),
                None => Err(Error::InvalidOpCode),
            },
        }
    }

    fn read_frame(&mut self, from_buffer: &[u8], to_buffer: &mut [u8]) -> Result<WebSocketFrame> {
        match &mut self.continuation_read {
            Some(continuation_read) => {
                let len_read = read_into_buffer(
                    continuation_read.mask_key,
                    from_buffer,
                    to_buffer,
                    continuation_read.count,
                );

                let is_complete = len_read == continuation_read.count;

                let frame = match continuation_read.op_code {
                    WebSocketOpCode::ConnectionClose => {
                        decode_close_frame(to_buffer, len_read, len_read)?
                    }
                    _ => WebSocketFrame {
                        num_bytes_from: len_read,
                        num_bytes_to: len_read,
                        op_code: continuation_read.op_code,
                        close_status: None,
                        is_fin_bit_set: if is_complete {
                            continuation_read.is_fin_bit_set
                        } else {
                            false
                        },
                    },
                };

                if is_complete {
                    self.continuation_read = None;
                } else {
                    continuation_read.count = continuation_read.count - len_read;
                }

                Ok(frame)
            }
            None => {
                if from_buffer.len() < 2 {
                    return Err(Error::ReadFromBufferTooSmall);
                }

                let byte1 = from_buffer[0];
                let byte2 = from_buffer[1];

                // process first byte
                const FIN_BIT_FLAG: u8 = 0x80;
                const OP_CODE_FLAG: u8 = 0x0F;
                let is_fin_bit_set = (byte1 & FIN_BIT_FLAG) == FIN_BIT_FLAG;
                let op_code = get_op_code(byte1 & OP_CODE_FLAG)?;

                // process second byte
                const MASK_FLAG: u8 = 0x80;
                let is_mask_bit_set = (byte2 & MASK_FLAG) == MASK_FLAG;
                let (len, mut num_bytes_read) = read_length(byte2, &from_buffer[2..])?;

                num_bytes_read = num_bytes_read + 2;
                let from_buffer = &from_buffer[num_bytes_read..];

                let mask_key = if is_mask_bit_set {
                    if from_buffer.len() < MASK_KEY_LEN {
                        return Err(Error::ReadFromBufferTooSmall);
                    }
                    let mut mask_key: [u8; MASK_KEY_LEN] = [0; MASK_KEY_LEN];
                    mask_key.copy_from_slice(&from_buffer[..MASK_KEY_LEN]);
                    num_bytes_read = num_bytes_read + MASK_KEY_LEN;
                    Some(mask_key)
                } else {
                    None
                };

                let len_read = if is_mask_bit_set {
                    let from_buffer = &from_buffer[MASK_KEY_LEN..];
                    read_into_buffer(mask_key, from_buffer, to_buffer, len)
                } else {
                    read_into_buffer(mask_key, from_buffer, to_buffer, len)
                };

                let has_continuation = len_read < len;
                num_bytes_read = num_bytes_read + len_read;

                let frame = match op_code {
                    WebSocketOpCode::ConnectionClose => {
                        decode_close_frame(to_buffer, num_bytes_read, len_read)?
                    }
                    _ => WebSocketFrame {
                        num_bytes_from: num_bytes_read,
                        num_bytes_to: len_read,
                        op_code,
                        close_status: None,
                        is_fin_bit_set: if has_continuation {
                            false
                        } else {
                            is_fin_bit_set
                        },
                    },
                };

                if has_continuation {
                    self.continuation_read = Some(ContinuationRead {
                        op_code,
                        count: len - len_read,
                        is_fin_bit_set,
                        mask_key,
                    });
                }

                Ok(frame)
            }
        }
    }
}

fn build_accept_string(sec_websocket_key: &str, output: &mut [u8]) -> Result<()> {
    // concatenate the key with a known websocket GUID (as per the spec)
    let mut accept_string: String<U64> = String::new();
    accept_string.push_str(sec_websocket_key)?;
    accept_string.push_str("258EAFA5-E914-47DA-95CA-C5AB0DC85B11")?;

    // calculate the base64 encoded sha1 hash of the accept string above
    let sha1 = Sha1::from(&accept_string);
    let input = sha1.digest().bytes();
    //let mut output = [0; 28];
    base64_encode(&input, output); // no need for slices since the output WILL be 28 bytes
    Ok(())
}

fn read_into_buffer(
    mask_key: Option<[u8; 4]>,
    from_buffer: &[u8],
    to_buffer: &mut [u8],
    len: usize,
) -> usize {
    let len_to_read = if len < to_buffer.len() {
        len
    } else {
        to_buffer.len()
    };
    let len_to_read = if len_to_read < from_buffer.len() {
        len_to_read
    } else {
        from_buffer.len()
    };

    match mask_key {
        Some(mask_key) => {
            let mut i = 0;
            for (from, to) in from_buffer[..len_to_read].iter().zip(to_buffer) {
                *to = *from ^ mask_key[i % MASK_KEY_LEN];
                i = i + 1;
            }
        }
        None => {
            to_buffer[..len_to_read].copy_from_slice(&from_buffer[..len_to_read]);
        }
    }

    len_to_read
}

fn write_frame(
    is_client: bool,
    from_buffer: &[u8],
    to_buffer: &mut [u8],
    op_code: WebSocketOpCode,
    end_of_message: bool,
    rng: &mut Random,
) -> Result<usize> {
    // we are working with byte array of 64 elements so all the unwraps below are ok
    let mut buf: Vec<u8, U64> = Vec::new();

    // write byte1
    let fin_bit_set_as_byte: u8 = if end_of_message { 0x80 } else { 0x00 };
    let byte1: u8 = fin_bit_set_as_byte | op_code as u8;
    buf.push(byte1).unwrap();
    let count = from_buffer.len();

    // write byte2 (and extra length bits if required)
    let mask_bit_set_as_byte = if is_client { 0x80 } else { 0x00 };
    if count < 126 {
        let byte2 = mask_bit_set_as_byte | count as u8;
        buf.push(byte2).unwrap();
    } else if count < 65535 {
        let byte2 = mask_bit_set_as_byte | 126;
        buf.push(byte2).unwrap();
        LittleEndian::write_u16(&mut buf, count as u16);
    } else {
        let byte2 = mask_bit_set_as_byte | 127;
        buf.push(byte2).unwrap();
        LittleEndian::write_u64(&mut buf, count as u64);
    }

    if is_client {
        // sent by client - need to mask the data
        // if this is a client then we need to mask the bytes to prevent web server caching
        let mut mask_key: [u8; 4] = [0; 4];
        rng.fill_bytes(&mut mask_key);
        buf.extend(&mask_key);

        to_buffer[..buf.len()].copy_from_slice(&buf);
        let to_buffer_start = buf.len();
        if (to_buffer_start + count) > to_buffer.len() {
            return Err(Error::WriteToBufferTooSmall);
        }

        let mut i = 0;
        for (from, to) in from_buffer[..count]
            .iter()
            .zip(&mut to_buffer[to_buffer_start..])
        {
            *to = *from ^ mask_key[i % MASK_KEY_LEN];
            i = i + 1;
        }
    } else {
        to_buffer[..buf.len()].copy_from_slice(&buf);

        if buf.len() + count > to_buffer.len() {
            return Err(Error::WriteToBufferTooSmall);
        }

        to_buffer[buf.len()..buf.len() + count].copy_from_slice(&from_buffer[..count]);
    }

    Ok(buf.len() + count)
}

fn get_op_code(val: u8) -> Result<WebSocketOpCode> {
    match val {
        0 => Ok(WebSocketOpCode::ContinuationFrame),
        1 => Ok(WebSocketOpCode::TextFrame),
        2 => Ok(WebSocketOpCode::BinaryFrame),
        8 => Ok(WebSocketOpCode::ConnectionClose),
        9 => Ok(WebSocketOpCode::Ping),
        10 => Ok(WebSocketOpCode::Pong),
        _ => Err(Error::InvalidOpCode),
    }
}

// returns (len, how_many_bytes_were_read)
fn read_length(byte2: u8, from_buffer: &[u8]) -> Result<(usize, usize)> {
    let len = byte2 & 0x7F;

    if len < 126 {
        return Ok((len as usize, 0));
    } else if len == 126 {
        if from_buffer.len() < 2 {
            return Err(Error::ReadFromBufferTooSmall);
        }
        let mut buf: [u8; 2] = [0; 2];
        buf.copy_from_slice(&from_buffer[..2]);
        return Ok((BigEndian::read_u16(&buf) as usize, 2));
    } else if len == 127 {
        if from_buffer.len() < 8 {
            return Err(Error::ReadFromBufferTooSmall);
        }
        let mut buf: [u8; 8] = [0; 8];
        buf.copy_from_slice(&from_buffer[..8]);
        return Ok((BigEndian::read_u64(&buf) as usize, 8));
    }

    Err(Error::InvalidOpCode)
}

fn decode_close_frame(
    buffer: &mut [u8],
    num_bytes_read: usize,
    len: usize,
) -> Result<WebSocketFrame> {
    if len >= 2 {
        // NOTE: for now, don't read the close status description
        let code = BigEndian::read_u16(buffer);
        let close_status_code = match code {
            1000 => Ok(WebSocketCloseStatusCode::NormalClosure),
            1001 => Ok(WebSocketCloseStatusCode::EndpointUnavailable),
            1002 => Ok(WebSocketCloseStatusCode::ProtocolError),
            1003 => Ok(WebSocketCloseStatusCode::InvalidMessageType),
            1005 => Ok(WebSocketCloseStatusCode::Empty),
            1007 => Ok(WebSocketCloseStatusCode::InvalidPayloadData),
            1008 => Ok(WebSocketCloseStatusCode::PolicyViolation),
            1009 => Ok(WebSocketCloseStatusCode::MessageTooBig),
            1010 => Ok(WebSocketCloseStatusCode::MandatoryExtension),
            1011 => Ok(WebSocketCloseStatusCode::InternalServerError),
            _ => Err(Error::InvalidCloseStatusCode),
        }?;

        return Ok(WebSocketFrame {
            num_bytes_from: num_bytes_read,
            num_bytes_to: len,
            op_code: WebSocketOpCode::ConnectionClose,
            close_status: Some(close_status_code),
            is_fin_bit_set: true,
        });
    }

    Ok(build_client_disconnected_frame(num_bytes_read))
}

fn build_client_disconnected_frame(num_bytes_from: usize) -> WebSocketFrame {
    WebSocketFrame {
        num_bytes_from,
        num_bytes_to: 0,
        op_code: WebSocketOpCode::ConnectionClose,
        close_status: Some(WebSocketCloseStatusCode::InternalServerError),
        is_fin_bit_set: true,
    }
}

// *************************************** BASE64 ENCODE ******************************************
// The base64_encode function below was adapted from the rust-base64 library
// https://github.com/alicemaz/rust-base64
// The MIT License (MIT)
// Copyright (c) 2015 Alice Maz, 2019 David Haig
// Adapted for no_std specifically for MIME (Standard) flavoured base64 encoding
// ************************************************************************************************

pub const BASE64_ENCODE_TABLE: &'static [u8; 64] = &[
    65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88,
    89, 90, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114,
    115, 116, 117, 118, 119, 120, 121, 122, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 43, 47,
];

fn base64_encode(input: &[u8], output: &mut [u8]) -> usize {
    let encode_table: &[u8; 64] = BASE64_ENCODE_TABLE;
    let mut input_index: usize = 0;
    let mut output_index = 0;
    const LOW_SIX_BITS_U8: u8 = 0x3F;
    let rem = input.len() % 3;
    let start_of_rem = input.len() - rem;

    while input_index < start_of_rem {
        let input_chunk = &input[input_index..(input_index + 3)];
        let output_chunk = &mut output[output_index..(output_index + 4)];

        output_chunk[0] = encode_table[(input_chunk[0] >> 2) as usize];
        output_chunk[1] =
            encode_table[((input_chunk[0] << 4 | input_chunk[1] >> 4) & LOW_SIX_BITS_U8) as usize];
        output_chunk[2] =
            encode_table[((input_chunk[1] << 2 | input_chunk[2] >> 6) & LOW_SIX_BITS_U8) as usize];
        output_chunk[3] = encode_table[(input_chunk[2] & LOW_SIX_BITS_U8) as usize];

        input_index += 3;
        output_index += 4;
    }

    if rem == 2 {
        output[output_index] = encode_table[(input[start_of_rem] >> 2) as usize];
        output[output_index + 1] = encode_table[((input[start_of_rem] << 4
            | input[start_of_rem + 1] >> 4)
            & LOW_SIX_BITS_U8) as usize];
        output[output_index + 2] =
            encode_table[((input[start_of_rem + 1] << 2) & LOW_SIX_BITS_U8) as usize];
        output_index += 3;
    } else if rem == 1 {
        output[output_index] = encode_table[(input[start_of_rem] >> 2) as usize];
        output[output_index + 1] =
            encode_table[((input[start_of_rem] << 4) & LOW_SIX_BITS_U8) as usize];
        output_index += 2;
    }

    // add padding
    let rem = input.len() % 3;
    for _ in 0..((3 - rem) % 3) {
        output[output_index] = b'=';
        output_index += 1;
    }

    output_index
}

// ************************************************************************************************
// **************************************** TESTS *************************************************
// ************************************************************************************************

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn opening_handshake() {
        let client_request = "GET /chat HTTP/1.1
Host: localhost:5000
User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:62.0) Gecko/20100101 Firefox/62.0
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8
Accept-Language: en-US,en;q=0.5
Accept-Encoding: gzip, deflate
Sec-WebSocket-Version: 13
Origin: http://localhost:5000
Sec-WebSocket-Extensions: permessage-deflate
Sec-WebSocket-Key: Z7OY1UwHOx/nkSz38kfPwg==
DNT: 1
Connection: keep-alive, Upgrade
Pragma: no-cache
Cache-Control: no-cache
Upgrade: websocket

";

        let http_header = read_http_header(&client_request.as_bytes()).unwrap();
        let web_socket_context = http_header.websocket_context.unwrap();
        let mut web_socket = WebSocket::new_server();
        let mut ws_buffer: [u8; 3000] = [0; 3000];
        let size = web_socket
            .server_respond_to_opening_handshake(
                &web_socket_context.sec_websocket_key,
                None,
                &mut ws_buffer,
            )
            .unwrap();
        let response = std::str::from_utf8(&ws_buffer[..size]).unwrap();
        let client_response_expected = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ptPnPeDOTo6khJlzmLhOZSh2tAY=\r\n\r\n";
        assert_eq!(client_response_expected, response);
    }

    // ASCII values A-Za-z0-9+/
    pub const STANDARD_ENCODE: &'static [u8; 64] = &[
        65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87,
        88, 89, 90, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112,
        113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
        43, 47,
    ];

    #[test]
    fn base64_encode_test() {
        let input: &[u8] = &[0; 20];
        let output: &mut [u8] = &mut [0; 100];
        let encode_table: &[u8; 64] = STANDARD_ENCODE;
        let mut input_index: usize = 0;
        let mut output_index = 0;

        const LOW_SIX_BITS_U8: u8 = 0x3F;

        let rem = input.len() % 3;
        let start_of_rem = input.len() - rem;

        while input_index < start_of_rem {
            let input_chunk = &input[input_index..(input_index + 3)];
            let output_chunk = &mut output[output_index..(output_index + 4)];

            output_chunk[0] = encode_table[(input_chunk[0] >> 2) as usize];
            output_chunk[1] = encode_table
                [((input_chunk[0] << 4 | input_chunk[1] >> 4) & LOW_SIX_BITS_U8) as usize];
            output_chunk[2] = encode_table
                [((input_chunk[1] << 2 | input_chunk[2] >> 6) & LOW_SIX_BITS_U8) as usize];
            output_chunk[3] = encode_table[(input_chunk[2] & LOW_SIX_BITS_U8) as usize];

            input_index += 3;
            output_index += 4;
        }
    }

    #[test]
    fn closing_handshake() {
        let mut ws_client_buffer: [u8; 500] = [0; 500];
        let mut ws_server_buffer: [u8; 500] = [0; 500];

        let mut ws_client = WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_server = WebSocket {
            is_client: false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        // client sends a close (initiates the close handshake)
        ws_client
            .close(
                WebSocketCloseStatusCode::NormalClosure,
                None,
                &mut ws_client_buffer,
            )
            .unwrap();

        // check that the client receives the close message
        let ws_result = ws_server
            .read(&ws_client_buffer, &mut ws_server_buffer)
            .unwrap();
        assert_eq!(
            WebSocketReceiveMessageType::CloseMustReply,
            ws_result.message_type
        );

        // server MUST respond to complete the handshake
        ws_server
            .write(
                WebSocketSendMessageType::CloseReply,
                true,
                &ws_server_buffer[..ws_result.num_bytes_to],
                &mut ws_client_buffer,
            )
            .unwrap();

        // check that the client receives the close message from the server
        let ws_result = ws_client
            .read(&ws_client_buffer, &mut ws_server_buffer)
            .unwrap();
        assert_eq!(
            WebSocketReceiveMessageType::CloseCompleted,
            ws_result.message_type
        );
    }

    #[test]
    fn send_message_from_client_to_server() {
        send_test_message(true);
    }

    #[test]
    fn send_message_from_server_to_client() {
        send_test_message(false);
    }

    fn send_test_message(from_client_to_server: bool) {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_from = WebSocket {
            is_client: from_client_to_server,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_to = WebSocket {
            is_client: !from_client_to_server,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        // client sends a Text message
        let hello = "hello";
        let num_bytes = ws_to
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        // check that the Server receives the Text message
        let ws_result = ws_from.read(&buffer1[..num_bytes], &mut buffer2).unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result.message_type);
        let received = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap();
        assert_eq!(hello, received);
    }

    #[test]
    fn receive_buffer_too_small() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_from = WebSocket {
            is_client: false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_to = WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let hello = "hello";
        ws_to
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        match ws_from.read(&buffer1[..1], &mut buffer2) {
            Err(Error::ReadFromBufferTooSmall) => {
                // test passes
            }
            _ => {
                assert_eq!(true, false);
            }
        }
    }

    #[test]
    fn receive_large_frame_with_small_receive_buffer() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_from = WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_to = WebSocket {
            is_client: false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let hello = "hello";
        ws_to
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        let ws_result = ws_from.read(&buffer1[..2], &mut buffer2).unwrap();
        assert_eq!(0, ws_result.num_bytes_to);
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_from.read(&buffer1[2..3], &mut buffer2).unwrap();
        assert_eq!(1, ws_result.num_bytes_to);
        assert_eq!(
            "h",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_from.read(&buffer1[3..], &mut buffer2).unwrap();
        assert_eq!(4, ws_result.num_bytes_to);
        assert_eq!(
            "ello",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(true, ws_result.end_of_message);
    }

    #[test]
    fn receive_large_frame_with_small_send_buffer() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_from = WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_to = WebSocket {
            is_client: false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let hello = "hello";
        ws_to
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        let ws_result = ws_from.read(&buffer1, &mut buffer2[..1]).unwrap();
        assert_eq!(1, ws_result.num_bytes_to);
        assert_eq!(
            "h",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_from
            .read(&buffer1[ws_result.num_bytes_from..], &mut buffer2[..4])
            .unwrap();
        assert_eq!(4, ws_result.num_bytes_to);
        assert_eq!(
            "ello",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(true, ws_result.end_of_message);
    }

    #[test]
    fn send_multi_frame_message() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_from = WebSocket {
            is_client: true,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        let mut ws_to = WebSocket {
            is_client: !false,
            rng: Random::new(0),
            continuation_frame_op_code: None,
            state: WebSocketState::Open,
            continuation_read: None,
        };

        // client sends a fragmented Text message
        let hello = "Hello, ";
        let num_bytes_hello = ws_to
            .write(
                WebSocketSendMessageType::Text,
                false,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        // client sends the remaining Text message
        let world = "World!";
        let num_bytes_world = ws_to
            .write(
                WebSocketSendMessageType::Text,
                true,
                &world.as_bytes(),
                &mut buffer1[num_bytes_hello..],
            )
            .unwrap();

        // check that the Server receives the entire Text message
        let ws_result1 = ws_from
            .read(&buffer1[..num_bytes_hello], &mut buffer2)
            .unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result1.message_type);
        assert_eq!(false, ws_result1.end_of_message);
        let ws_result2 = ws_from
            .read(
                &buffer1[num_bytes_hello..num_bytes_hello + num_bytes_world],
                &mut buffer2[ws_result1.num_bytes_to..],
            )
            .unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result2.message_type);
        assert_eq!(true, ws_result2.end_of_message);

        let received =
            std::str::from_utf8(&buffer2[..ws_result1.num_bytes_to + ws_result2.num_bytes_to])
                .unwrap();
        assert_eq!("Hello, World!", received);
    }
}
