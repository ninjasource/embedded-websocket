// This module helps you work with the websocket library using a stream of data rather than using it as a raw codec.
// This is the most common use case when working with websockets and is recommended due to the hand shaky nature of
// the protocol as well as the fact that an input buffer can contain multiple websocket frames or maybe only a fragment of one.
// This module allows you to work with discrete websocket frames rather than the multiple fragments you read off a stream.
// NOTE: if you are using the standard library then you can use the built in Read and Write traits from std otherwise
//       you have to implement the Read and Write traits specified below

use crate::{
    WebSocket, WebSocketCloseStatusCode, WebSocketOptions, WebSocketReceiveMessageType,
    WebSocketSendMessageType, WebSocketType,
};
use core::{cmp::min, str::Utf8Error};
use rand_core::RngCore;

#[cfg(feature = "std")]
use std::io::{Read, Write};

#[cfg(feature = "std")]
use std::io::Error as IoError;

#[cfg(not(feature = "std"))]
#[derive(Debug)]
pub enum IoError {
    ReadError,
    WriteError,
}

#[cfg(not(feature = "std"))]
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;
}

#[cfg(not(feature = "std"))]
pub trait Write {
    fn write_all(&mut self, buf: &[u8]) -> Result<(), IoError>;
}

#[derive(Debug)]
pub enum FramerError {
    Io(IoError),
    FrameTooLarge(usize),
    Utf8(Utf8Error),
    WebSocket(crate::Error),
}

impl From<IoError> for FramerError {
    fn from(err: IoError) -> Self {
        FramerError::Io(err)
    }
}

impl From<Utf8Error> for FramerError {
    fn from(err: Utf8Error) -> Self {
        FramerError::Utf8(err)
    }
}

impl From<crate::Error> for FramerError {
    fn from(err: crate::Error) -> Self {
        FramerError::WebSocket(err)
    }
}

pub struct Framer<'a, TRng, TWebSocketType, TStream>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
    TStream: Read + Write,
{
    read_buf: &'a mut [u8],
    write_buf: &'a mut [u8],
    read_cursor: usize,
    frame_cursor: usize,
    read_len: usize,
    websocket: &'a mut WebSocket<TRng, TWebSocketType>,
    stream: &'a mut TStream,
}

impl<'a, TRng, TStream> Framer<'a, TRng, crate::Client, TStream>
where
    TRng: RngCore,
    TStream: Read + Write,
{
    pub fn connect(&mut self, websocket_options: &WebSocketOptions) -> Result<(), FramerError> {
        let (len, web_socket_key) = self
            .websocket
            .client_connect(&websocket_options, &mut self.write_buf)?;
        self.stream.write_all(&self.write_buf[..len])?;

        // read the response from the server and check it to complete the opening handshake
        let received_size = self.stream.read(&mut self.read_buf)?;
        self.websocket
            .client_accept(&web_socket_key, &self.read_buf[..received_size])?;
        Ok(())
    }
}

impl<'a, TRng, TWebSocketType, TStream> Framer<'a, TRng, TWebSocketType, TStream>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
    TStream: Read + Write,
{
    // read and write buffers are usually quite small (4KB) and can be smaller
    // than the frame buffer but use whatever is is appropriate for your stream
    pub fn new(
        read_buf: &'a mut [u8],
        write_buf: &'a mut [u8],
        websocket: &'a mut WebSocket<TRng, TWebSocketType>,
        stream: &'a mut TStream,
    ) -> Self {
        Self {
            read_buf,
            write_buf,
            read_cursor: 0,
            frame_cursor: 0,
            read_len: 0,
            websocket,
            stream,
        }
    }

    // calling close on a websocket that has already been closed by the other party has no effect
    pub fn close(
        &mut self,
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
    ) -> Result<(), FramerError> {
        let len = self
            .websocket
            .close(close_status, status_description, self.write_buf)?;
        self.stream.write_all(&self.write_buf[..len])?;
        Ok(())
    }

    pub fn write(
        &mut self,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        frame_buf: &[u8],
    ) -> Result<(), FramerError> {
        let len = self
            .websocket
            .write(message_type, end_of_message, frame_buf, self.write_buf)?;
        self.stream.write_all(&self.write_buf[..len])?;
        Ok(())
    }

    // frame_buf should be large enough to hold an entire websocket text frame
    // this function will block until it has recieved a full websocket frame.
    // It will wait until the last fragmented frame has arrived.
    pub fn read_text<'b>(
        &mut self,
        frame_buf: &'b mut [u8],
    ) -> Result<Option<&'b str>, FramerError> {
        if let Some(frame) = self.next(frame_buf, WebSocketReceiveMessageType::Text)? {
            Ok(Some(core::str::from_utf8(frame)?))
        } else {
            Ok(None)
        }
    }

    // frame_buf should be large enough to hold an entire websocket binary frame
    // this function will block until it has recieved a full websocket frame.
    // It will wait until the last fragmented frame has arrived.
    pub fn read_binary<'b>(
        &mut self,
        frame_buf: &'b mut [u8],
    ) -> Result<Option<&'b [u8]>, FramerError> {
        self.next(frame_buf, WebSocketReceiveMessageType::Binary)
    }

    fn next<'b>(
        &mut self,
        frame_buf: &'b mut [u8],
        message_type: WebSocketReceiveMessageType,
    ) -> Result<Option<&'b [u8]>, FramerError> {
        loop {
            if self.read_cursor == 0 || self.read_cursor == self.read_len {
                self.read_len = self.stream.read(self.read_buf)?;
                self.read_cursor = 0;
            }

            if self.read_len == 0 {
                return Ok(None);
            }

            loop {
                if self.read_cursor == self.read_len {
                    break;
                }

                if self.frame_cursor == frame_buf.len() {
                    return Err(FramerError::FrameTooLarge(frame_buf.len()));
                }

                let ws_result = self.websocket.read(
                    &self.read_buf[self.read_cursor..self.read_len],
                    &mut frame_buf[self.frame_cursor..],
                )?;

                self.read_cursor += ws_result.len_from;

                match ws_result.message_type {
                    // text or binary frame
                    x if x == message_type => {
                        self.frame_cursor += ws_result.len_to;
                        if ws_result.end_of_message {
                            let frame = &frame_buf[..self.frame_cursor];
                            self.frame_cursor = 0;
                            return Ok(Some(frame));
                        }
                    }
                    WebSocketReceiveMessageType::CloseMustReply => {
                        self.send_back(
                            frame_buf,
                            ws_result.len_to,
                            WebSocketSendMessageType::CloseReply,
                        )?;
                        return Ok(None);
                    }
                    WebSocketReceiveMessageType::CloseCompleted => return Ok(None),
                    WebSocketReceiveMessageType::Ping => {
                        self.send_back(
                            frame_buf,
                            ws_result.len_to,
                            WebSocketSendMessageType::Pong,
                        )?;
                    }
                    // ignore other message types
                    _ => {}
                }
            }
        }
    }

    fn send_back(
        &mut self,
        frame_buf: &'_ mut [u8],
        len_to: usize,
        send_message_type: WebSocketSendMessageType,
    ) -> Result<(), FramerError> {
        let payload_len = min(self.write_buf.len(), len_to);
        let from = &frame_buf[self.frame_cursor..self.frame_cursor + payload_len];
        let len = self
            .websocket
            .write(send_message_type, true, from, &mut self.write_buf)?;
        self.stream.write_all(&self.write_buf[..len])?;
        Ok(())
    }
}
