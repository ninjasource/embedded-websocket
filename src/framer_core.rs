use core::ops::Range;

use rand_core::RngCore;

use crate::{
    framer::{FramerError, ReadResult},
    WebSocket, WebSocketCloseStatusCode, WebSocketContext, WebSocketKey, WebSocketOptions,
    WebSocketReceiveMessageType, WebSocketSendMessageType, WebSocketState, WebSocketSubProtocol,
    WebSocketType,
};

pub(crate) enum Decoded {
    NeedMoreData,
    SendThenContinue(usize),
    SendThenClosed(usize),
    Message(Message),
}

pub(crate) enum Message {
    Binary(usize),
    Text(usize),
    Pong(Range<usize>),
    Closed,
}

impl Message {
    pub(crate) fn into_read_result<E>(
        self,
        frame_buf: &[u8],
    ) -> Result<ReadResult<'_>, FramerError<E>> {
        Ok(match self {
            Message::Binary(len) => ReadResult::Binary(&frame_buf[..len]),
            Message::Text(len) => ReadResult::Text(
                core::str::from_utf8(&frame_buf[..len]).map_err(FramerError::Utf8)?,
            ),
            Message::Pong(range) => ReadResult::Pong(&frame_buf[range]),
            Message::Closed => ReadResult::Closed,
        })
    }
}

pub(crate) struct FramerCore<'a, TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    read_buf: &'a mut [u8],
    // Undecoded bytes always start at index 0, so a framer recreated with the same `read_buf` and
    // `read_len` continues where the previous one stopped.
    read_len: &'a mut usize,
    write_buf: &'a mut [u8],
    frame_cursor: usize,
    websocket: &'a mut WebSocket<TRng, TWebSocketType>,
}

impl<'a, TRng> FramerCore<'a, TRng, crate::Client>
where
    TRng: RngCore,
{
    pub(crate) fn encode_connect<E>(
        &mut self,
        websocket_options: &WebSocketOptions,
    ) -> Result<(usize, WebSocketKey), FramerError<E>> {
        *self.read_len = 0;
        self.websocket
            .client_connect(websocket_options, self.write_buf)
            .map_err(FramerError::WebSocket)
    }

    pub(crate) fn decode_connect_response<E>(
        &mut self,
        web_socket_key: &WebSocketKey,
    ) -> Result<Option<Option<WebSocketSubProtocol>>, FramerError<E>> {
        match self
            .websocket
            .client_accept(web_socket_key, &self.read_buf[..*self.read_len])
        {
            Ok((len, sub_protocol)) => {
                self.consume(len);
                Ok(Some(sub_protocol))
            }
            Err(crate::Error::HttpHeaderIncomplete) => Ok(None),
            Err(e) => Err(FramerError::WebSocket(e)),
        }
    }

    pub(crate) fn connect_read_space<E>(&mut self) -> Result<&mut [u8], FramerError<E>> {
        if *self.read_len == self.read_buf.len() {
            return Err(FramerError::WebSocket(crate::Error::HttpHeaderIncomplete));
        }
        Ok(&mut self.read_buf[*self.read_len..])
    }
}

impl<'a, TRng> FramerCore<'a, TRng, crate::Server>
where
    TRng: RngCore,
{
    pub(crate) fn encode_accept<E>(
        &mut self,
        websocket_context: &WebSocketContext,
    ) -> Result<usize, FramerError<E>> {
        self.websocket
            .server_accept(&websocket_context.sec_websocket_key, None, self.write_buf)
            .map_err(FramerError::WebSocket)
    }
}

impl<'a, TRng, TWebSocketType> FramerCore<'a, TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    pub(crate) fn new(
        read_buf: &'a mut [u8],
        read_len: &'a mut usize,
        write_buf: &'a mut [u8],
        websocket: &'a mut WebSocket<TRng, TWebSocketType>,
    ) -> Self {
        Self {
            read_buf,
            read_len,
            write_buf,
            frame_cursor: 0,
            websocket,
        }
    }

    pub(crate) fn state(&self) -> WebSocketState {
        self.websocket.state
    }

    pub(crate) fn written(&self, len: usize) -> &[u8] {
        &self.write_buf[..len]
    }

    pub(crate) fn encode_close<E>(
        &mut self,
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
    ) -> Result<usize, FramerError<E>> {
        self.websocket
            .close(close_status, status_description, self.write_buf)
            .map_err(FramerError::WebSocket)
    }

    pub(crate) fn encode_write<E>(
        &mut self,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        frame_buf: &[u8],
    ) -> Result<usize, FramerError<E>> {
        self.websocket
            .write(message_type, end_of_message, frame_buf, self.write_buf)
            .map_err(FramerError::WebSocket)
    }

    pub(crate) fn read_space<E>(&mut self) -> Result<&mut [u8], FramerError<E>> {
        if *self.read_len == self.read_buf.len() {
            return Err(FramerError::FrameTooLarge(self.read_buf.len()));
        }
        Ok(&mut self.read_buf[*self.read_len..])
    }

    pub(crate) fn commit_read(&mut self, len: usize) {
        *self.read_len += len;
    }

    pub(crate) fn decode<E>(&mut self, frame_buf: &mut [u8]) -> Result<Decoded, FramerError<E>> {
        loop {
            let buffered = &self.read_buf[..*self.read_len];
            if buffered.is_empty() {
                return Ok(Decoded::NeedMoreData);
            }

            // Control frames are decoded in one piece because their payload is answered or
            // returned as a whole and the close status is only parsed at the start of a frame.
            match self.websocket.next_control_frame_len(buffered) {
                Ok(Some(len)) if len.total > buffered.len() => {
                    return self.need_more_data();
                }
                Ok(Some(len)) if self.frame_cursor + len.payload > frame_buf.len() => {
                    return Err(FramerError::FrameTooLarge(frame_buf.len()));
                }
                Ok(_) => {}
                Err(crate::Error::ReadFrameIncomplete) => return self.need_more_data(),
                Err(e) => return Err(FramerError::WebSocket(e)),
            }

            if self.frame_cursor >= frame_buf.len() {
                return Err(FramerError::FrameTooLarge(frame_buf.len()));
            }

            let ws_result = match self
                .websocket
                .read(buffered, &mut frame_buf[self.frame_cursor..])
            {
                Ok(ws_result) => ws_result,
                Err(crate::Error::ReadFrameIncomplete) => return self.need_more_data(),
                Err(e) => return Err(FramerError::WebSocket(e)),
            };
            self.consume(ws_result.len_from);

            let payload = self.frame_cursor..self.frame_cursor + ws_result.len_to;
            match ws_result.message_type {
                WebSocketReceiveMessageType::Binary | WebSocketReceiveMessageType::Text => {
                    self.frame_cursor = payload.end;
                    if ws_result.end_of_message {
                        self.frame_cursor = 0;
                        return Ok(Decoded::Message(
                            if ws_result.message_type == WebSocketReceiveMessageType::Binary {
                                Message::Binary(payload.end)
                            } else {
                                Message::Text(payload.end)
                            },
                        ));
                    }
                }
                WebSocketReceiveMessageType::CloseMustReply => {
                    let len = self.encode_write(
                        WebSocketSendMessageType::CloseReply,
                        true,
                        &frame_buf[payload],
                    )?;
                    return Ok(Decoded::SendThenClosed(len));
                }
                WebSocketReceiveMessageType::CloseCompleted => {
                    return Ok(Decoded::Message(Message::Closed));
                }
                WebSocketReceiveMessageType::Ping => {
                    let len = self.encode_write(
                        WebSocketSendMessageType::Pong,
                        true,
                        &frame_buf[payload],
                    )?;
                    return Ok(Decoded::SendThenContinue(len));
                }
                WebSocketReceiveMessageType::Pong => {
                    return Ok(Decoded::Message(Message::Pong(payload)));
                }
            }
        }
    }

    fn need_more_data<E>(&self) -> Result<Decoded, FramerError<E>> {
        if *self.read_len == self.read_buf.len() {
            Err(FramerError::FrameTooLarge(self.read_buf.len()))
        } else {
            Ok(Decoded::NeedMoreData)
        }
    }

    fn consume(&mut self, len: usize) {
        self.read_buf.copy_within(len..*self.read_len, 0);
        *self.read_len -= len;
    }
}
