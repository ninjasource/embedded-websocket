// The MIT License (MIT)
// Copyright (c) 2019 David Haig

// This library facilitates the encoding and decoding of websocket frames and can be used
// for both clients and servers. The library is intended to be used in constrained memory
// environments like embedded microcontrollers which cannot reference the rust standard library.
// It will work with arbitrarily small buffers regardless of websocket frame size as long as the
// websocket header can be read (2 - 14 bytes depending)
// Since the library is essentially an encoder or decoder of byte slices, the developer is free to
// use whatever transport mechanism they chose. The examples use the TcpStream from the standard
// library.

#![no_std]

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use core::{cmp, result, str};
use heapless::consts::*; // these are for constants like U4, U16, U24, U32
use heapless::{String, Vec};
use rand_core::RngCore;
use sha1::Sha1;

mod base64;
mod http;
mod random;
pub use self::http::{read_http_header, HttpHeader, WebSocketContext};

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum WebSocketSendMessageType {
    Text = 1,
    Binary = 2,
    Ping = 9,
    Pong = 10,
    CloseReply = 11,
}

impl WebSocketSendMessageType {
    fn to_op_code(&self) -> WebSocketOpCode {
        match self {
            WebSocketSendMessageType::Text => WebSocketOpCode::TextFrame,
            WebSocketSendMessageType::Binary => WebSocketOpCode::BinaryFrame,
            WebSocketSendMessageType::Ping => WebSocketOpCode::Ping,
            WebSocketSendMessageType::Pong => WebSocketOpCode::Pong,
            WebSocketSendMessageType::CloseReply => WebSocketOpCode::ConnectionClose,
        }
    }
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
pub type WebSocketKey = String<U24>;
pub type WebSocketSubProtocol = String<U24>;

use random::EmptyRng;

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
    ConvertInfallible,
    RandCore,
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
impl From<core::convert::Infallible> for Error {
    fn from(_: core::convert::Infallible) -> Error {
        Error::ConvertInfallible
    }
}

impl From<()> for Error {
    fn from(_: ()) -> Error {
        Error::Unknown
    }
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

impl WebSocketFrame {
    fn to_readresult(&self, message_type: WebSocketReceiveMessageType) -> WebSocketReadResult {
        WebSocketReadResult {
            num_bytes_from: self.num_bytes_from,
            num_bytes_to: self.num_bytes_to,
            end_of_message: self.is_fin_bit_set,
            close_status: self.close_status,
            message_type,
        }
    }
}

// this is used when we cannot fit the entire websocket frame into the supplied buffer
pub struct ContinuationRead {
    op_code: WebSocketOpCode,
    count: usize,
    is_fin_bit_set: bool,
    mask_key: Option<[u8; 4]>,
}

pub struct WebSocketOptions<'a> {
    pub path: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub sub_protocols: Option<&'a [&'a str]>,
    pub additional_headers: Option<&'a [&'a str]>,
}

pub type WebSocketServer = WebSocket<EmptyRng>;

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
    pub fn new_client(rng: T) -> WebSocket<T> {
        WebSocket {
            is_client: true,
            rng,
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }

    // NOTE: this should be called as follows:
    // WebSocketServer::new_server()
    // or you will get a "type annotations needed" error
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

    // TODO: find out why this does not work:
    // let mut ws = WebSocket::new_server_not_working();
    pub fn new_server_not_working() -> WebSocket<EmptyRng> {
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
        if self.is_client {
            panic!("Server websocket expected");
        }

        match http::build_connect_handshake_response(sec_websocket_key, sec_websocket_protocol, to)
        {
            Ok(http_response_len) => {
                self.state = WebSocketState::Open;
                Ok(http_response_len)
            }
            Err(e) => {
                self.state = WebSocketState::Connecting;
                Err(e)
            }
        }
    }

    pub fn client_connect(
        &mut self,
        websocket_options: &WebSocketOptions,
        to: &mut [u8],
    ) -> Result<(usize, WebSocketKey)> {
        if !self.is_client {
            panic!("Client websocket expected");
        }

        match http::build_connect_handshake_request(websocket_options, &mut self.rng, to) {
            Ok((request_len, sec_websocket_key)) => {
                self.state = WebSocketState::Connecting;
                Ok((request_len, sec_websocket_key))
            }
            Err(e) => Err(e),
        }
    }

    pub fn client_accept(
        &mut self,
        sec_websocket_key: &WebSocketKey,
        from: &[u8],
    ) -> Result<Option<WebSocketSubProtocol>> {
        if !self.is_client {
            panic!("Client websocket expected");
        }

        match http::read_server_connect_handshake_response(sec_websocket_key, from) {
            Ok(sec_websocket_protocol) => {
                self.state = WebSocketState::Open;
                Ok(sec_websocket_protocol)
            }
            Err(e) => {
                self.state = WebSocketState::Aborted;
                Err(e)
            }
        }
    }

    pub fn read(&mut self, from: &[u8], to: &mut [u8]) -> Result<WebSocketReadResult> {
        let frame = self.read_frame(from, to)?;

        match frame.op_code {
            WebSocketOpCode::Ping => Ok(frame.to_readresult(WebSocketReceiveMessageType::Ping)),
            WebSocketOpCode::Pong => Ok(frame.to_readresult(WebSocketReceiveMessageType::Pong)),
            WebSocketOpCode::TextFrame => {
                Ok(frame.to_readresult(WebSocketReceiveMessageType::Text))
            }
            WebSocketOpCode::BinaryFrame => {
                Ok(frame.to_readresult(WebSocketReceiveMessageType::Binary))
            }
            WebSocketOpCode::ConnectionClose => match self.state {
                WebSocketState::CloseSent => {
                    self.state = WebSocketState::Closed;
                    Ok(frame.to_readresult(WebSocketReceiveMessageType::CloseCompleted))
                }
                _ => {
                    self.state = WebSocketState::CloseReceived;
                    Ok(frame.to_readresult(WebSocketReceiveMessageType::CloseMustReply))
                }
            },
            WebSocketOpCode::ContinuationFrame => match self.continuation_frame_op_code {
                Some(cf_op_code) => Ok(frame.to_readresult(cf_op_code.to_message_type()?)),
                None => Err(Error::InvalidOpCode),
            },
        }
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
        let fin_bit_set_as_byte: u8 = if end_of_message { 0x80 } else { 0x00 };
        let byte1: u8 = fin_bit_set_as_byte | op_code as u8;
        let count = from_buffer.len();
        const BYTE_HEADER_SIZE: usize = 2;
        const SHORT_HEADER_SIZE: usize = 4;
        const LONG_HEADER_SIZE: usize = 10;
        const MASK_KEY_SIZE: usize = 4;
        let header_size;
        let mask_bit_set_as_byte = if self.is_client { 0x80 } else { 0x00 };
        let payload_len = from_buffer.len() + if self.is_client { MASK_KEY_SIZE } else { 0 };

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

fn min(num1: usize, num2: usize, num3: usize) -> usize {
    cmp::min(cmp::min(num1, num2), num3)
}

fn read_into_buffer(
    mask_key: Option<[u8; 4]>,
    from_buffer: &[u8],
    to_buffer: &mut [u8],
    len: usize,
) -> usize {
    // if we are trying to read more than number of bytes in either buffer
    let len_to_read = min(len, to_buffer.len(), from_buffer.len());

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

fn read_continuation(
    continuation_read: &mut ContinuationRead,
    from_buffer: &[u8],
    to_buffer: &mut [u8],
) -> Result<WebSocketFrame> {
    let len_read = read_into_buffer(
        continuation_read.mask_key,
        from_buffer,
        to_buffer,
        continuation_read.count,
    );

    let is_complete = len_read == continuation_read.count;

    let frame = match continuation_read.op_code {
        WebSocketOpCode::ConnectionClose => decode_close_frame(to_buffer, len_read, len_read)?,
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

    continuation_read.count = continuation_read.count - len_read;
    Ok(frame)
}

fn read_frame(
    from_buffer: &[u8],
    to_buffer: &mut [u8],
) -> Result<(WebSocketFrame, Option<ContinuationRead>)> {
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
        let continuation_read = Some(ContinuationRead {
            op_code,
            count: len - len_read,
            is_fin_bit_set,
            mask_key,
        });
        Ok((frame, continuation_read))
    } else {
        Ok((frame, None))
    }
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
        let mut web_socket = WebSocketServer::new_server();

        let mut ws_buffer: [u8; 3000] = [0; 3000];
        let size = web_socket
            .server_accept(&web_socket_context.sec_websocket_key, None, &mut ws_buffer)
            .unwrap();
        let response = std::str::from_utf8(&ws_buffer[..size]).unwrap();
        let client_response_expected = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ptPnPeDOTo6khJlzmLhOZSh2tAY=\r\n\r\n";
        assert_eq!(client_response_expected, response);
    }

    #[test]
    fn closing_handshake() {
        let mut buffer1: [u8; 500] = [0; 500];
        let mut buffer2: [u8; 500] = [0; 500];

        let mut rng = rand::thread_rng();

        let mut ws_client = WebSocket::new_client(&mut rng);
        ws_client.state = WebSocketState::Open;

        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        // client sends a close (initiates the close handshake)
        ws_client
            .close(WebSocketCloseStatusCode::NormalClosure, None, &mut buffer1)
            .unwrap();

        // check that the client receives the close message
        let ws_result = ws_server.read(&buffer1, &mut buffer2).unwrap();
        assert_eq!(
            WebSocketReceiveMessageType::CloseMustReply,
            ws_result.message_type
        );

        // server MUST respond to complete the handshake
        ws_server
            .write(
                WebSocketSendMessageType::CloseReply,
                true,
                &buffer2[..ws_result.num_bytes_to],
                &mut buffer1,
            )
            .unwrap();
        assert_eq!(WebSocketState::Closed, ws_server.state);

        // check that the client receives the close message from the server
        let ws_result = ws_client.read(&buffer1, &mut buffer2).unwrap();
        assert_eq!(WebSocketState::Closed, ws_client.state);

        assert_eq!(
            WebSocketReceiveMessageType::CloseCompleted,
            ws_result.message_type
        );
    }

    #[test]
    fn send_message_from_client_to_server() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        // how to create a client
        let mut ws_client = WebSocket::new_client(rand::thread_rng());

        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        // client sends a Text message
        let hello = "hello";
        let num_bytes = ws_client
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        // check that the Server receives the Text message
        let ws_result = ws_server.read(&buffer1[..num_bytes], &mut buffer2).unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result.message_type);
        let received = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap();
        assert_eq!(hello, received);
    }

    #[test]
    fn send_message_from_server_to_client() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_client = WebSocket::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        // server sends a Text message
        let hello = "hello";
        let num_bytes = ws_server
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        // check that the client receives the Text message
        let ws_result = ws_client.read(&buffer1[..num_bytes], &mut buffer2).unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result.message_type);
        let received = std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap();
        assert_eq!(hello, received);
    }

    #[test]
    fn receive_buffer_too_small() {
        let mut buffer1: [u8; 1000] = [0; 1000];
        let mut buffer2: [u8; 1000] = [0; 1000];

        let mut ws_client = WebSocket::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        let hello = "hello";
        ws_server
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        match ws_client.read(&buffer1[..1], &mut buffer2) {
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

        let mut ws_client = WebSocket::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        let hello = "hello";
        ws_server
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        let ws_result = ws_client.read(&buffer1[..2], &mut buffer2).unwrap();
        assert_eq!(0, ws_result.num_bytes_to);
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_client.read(&buffer1[2..3], &mut buffer2).unwrap();
        assert_eq!(1, ws_result.num_bytes_to);
        assert_eq!(
            "h",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_client.read(&buffer1[3..], &mut buffer2).unwrap();
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

        let mut ws_client = WebSocket::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        let hello = "hello";
        ws_server
            .write(
                WebSocketSendMessageType::Text,
                true,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        let ws_result = ws_client.read(&buffer1, &mut buffer2[..1]).unwrap();
        assert_eq!(1, ws_result.num_bytes_to);
        assert_eq!(
            "h",
            std::str::from_utf8(&buffer2[..ws_result.num_bytes_to]).unwrap()
        );
        assert_eq!(false, ws_result.end_of_message);
        let ws_result = ws_client
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
        // let mut rng = rand::thread_rng();

        let mut ws_client = WebSocket::new_client(rand::thread_rng());
        ws_client.state = WebSocketState::Open;
        let mut ws_server = WebSocketServer::new_server();
        ws_server.state = WebSocketState::Open;

        // client sends a fragmented Text message
        let hello = "Hello, ";
        let num_bytes_hello = ws_server
            .write(
                WebSocketSendMessageType::Text,
                false,
                &hello.as_bytes(),
                &mut buffer1,
            )
            .unwrap();

        // client sends the remaining Text message
        let world = "World!";
        let num_bytes_world = ws_server
            .write(
                WebSocketSendMessageType::Text,
                true,
                &world.as_bytes(),
                &mut buffer1[num_bytes_hello..],
            )
            .unwrap();

        // check that the Server receives the entire Text message
        let ws_result1 = ws_client
            .read(&buffer1[..num_bytes_hello], &mut buffer2)
            .unwrap();
        assert_eq!(WebSocketReceiveMessageType::Text, ws_result1.message_type);
        assert_eq!(false, ws_result1.end_of_message);
        let ws_result2 = ws_client
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
