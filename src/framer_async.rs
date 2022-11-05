use core::{
    fmt::Debug,
    ops::{Deref, Range},
    str::Utf8Error,
};

use futures::{Sink, SinkExt, Stream, StreamExt};
use rand_core::RngCore;

use crate::{
    WebSocket, WebSocketCloseStatusCode, WebSocketOptions, WebSocketReceiveMessageType,
    WebSocketSendMessageType, WebSocketSubProtocol, WebSocketType,
};

pub enum ReadResult {
    Binary(Range<usize>),
    Text(Range<usize>),
    /// We received a pong message in response to a ping message we sent earlier.
    /// This should contain the same data as was sent in the ping message
    Pong(Range<usize>),
    /// We received a ping message from the other end
    /// A pong message with the same content will automatically be sent back as per the websocket spec
    /// However, this message is exposed to you so that you can see what is in the ping message
    Ping(Range<usize>),
    /// The other end initiated a close handshake. The contents of this message is usually the reason for the close
    /// A close response message will automatically be sent back to the other end to complete the close handshake
    /// However, this message is exposed to you so that you can see why
    Close(Range<usize>),
}

#[derive(Debug)]
pub enum FramerError<E> {
    Io(E),
    FrameTooLarge(usize),
    Utf8(Utf8Error),
    HttpHeader(httparse::Error),
    WebSocket(crate::Error),
    Disconnected,
    RxBufferTooSmall(usize),
}

pub struct Framer<TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    websocket: WebSocket<TRng, TWebSocketType>,
    frame_cursor: usize,
    rx_buf_range: Range<usize>,
}

impl<TRng> Framer<TRng, crate::Client>
where
    TRng: RngCore,
{
    pub async fn connect<'a, B, E>(
        &mut self,
        stream: &mut (impl Stream<Item = Result<B, E>> + Sink<&'a [u8], Error = E> + Unpin),
        rx_buf: &mut [u8],
        tx_buf: &'a mut [u8],
        websocket_options: &WebSocketOptions<'_>,
    ) -> Result<Option<WebSocketSubProtocol>, FramerError<E>>
    where
        B: AsRef<[u8]>,
    {
        let (len, web_socket_key) = self
            .websocket
            .client_connect(websocket_options, tx_buf.as_mut())
            .map_err(FramerError::WebSocket)?;

        let to_send = &tx_buf[..len];
        stream.send(to_send).await.map_err(FramerError::Io)?;
        stream.flush().await.map_err(FramerError::Io)?;

        if let Some(buf) = stream.next().await {
            let buf = buf.map_err(FramerError::Io)?;
            let buf = buf.as_ref();

            match self.websocket.client_accept(&web_socket_key, buf) {
                Ok((len, sub_protocol)) => {
                    // "consume" the HTTP header that we have read from the stream
                    // read_cursor would be 0 if we exactly read the HTTP header from the stream and nothing else

                    // copy the remaining bytes into the frame buffer because they are the contents of the next websocket frame(s)
                    let from = len;
                    let to = buf.len();
                    let remaining_len = to - from;
                    rx_buf[..remaining_len].copy_from_slice(&buf[from..to]);
                    self.rx_buf_range = 0..remaining_len;

                    return Ok(sub_protocol);
                }
                Err(crate::Error::HttpHeaderIncomplete) => {
                    // TODO: continue reading HTTP header in loop
                    panic!("oh no");
                }
                Err(e) => {
                    return Err(FramerError::WebSocket(e));
                }
            }
        }

        return Err(FramerError::Disconnected);
    }
}

impl<TRng, TWebSocketType> Framer<TRng, TWebSocketType>
where
    TRng: RngCore,
    TWebSocketType: WebSocketType,
{
    pub fn new(websocket: WebSocket<TRng, TWebSocketType>) -> Self {
        Self {
            websocket,
            frame_cursor: 0,
            rx_buf_range: 0..0,
        }
    }

    pub fn encode<'b, E>(
        &mut self,
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        from: &[u8],
        to: &'b mut [u8],
    ) -> Result<usize, FramerError<E>> {
        let len = self
            .websocket
            .write(message_type, end_of_message, from, to)
            .map_err(FramerError::WebSocket)?;

        Ok(len)
    }

    pub async fn write<'b, E>(
        &mut self,
        tx: &mut (impl Sink<&'b [u8], Error = E> + Unpin),
        tx_buf: &'b mut [u8],
        message_type: WebSocketSendMessageType,
        end_of_message: bool,
        frame_buf: &[u8],
    ) -> Result<(), FramerError<E>>
    where
        E: Debug,
    {
        let len = self
            .websocket
            .write(message_type, end_of_message, frame_buf, tx_buf)
            .map_err(FramerError::WebSocket)?;

        tx.send(&tx_buf[..len]).await.map_err(FramerError::Io)?;
        tx.flush().await.map_err(FramerError::Io)?;
        Ok(())
    }

    pub async fn close<'b, E>(
        &mut self,
        tx: &mut (impl Sink<&'b [u8], Error = E> + Unpin),
        tx_buf: &'b mut [u8],
        close_status: WebSocketCloseStatusCode,
        status_description: Option<&str>,
    ) -> Result<(), FramerError<E>>
    where
        E: Debug,
    {
        let len = self
            .websocket
            .close(close_status, status_description, tx_buf)
            .map_err(FramerError::WebSocket)?;

        tx.send(&tx_buf[..len]).await.map_err(FramerError::Io)?;
        tx.flush().await.map_err(FramerError::Io)?;
        Ok(())
    }

    pub async fn read<'a, B: Deref<Target = [u8]>, E>(
        &mut self,
        stream: &mut (impl Stream<Item = Result<B, E>> + Sink<&'a [u8], Error = E> + Unpin),
        rx_buf: &'a mut [u8],
        tx_buf: &'a mut [u8],
        frame_buf: &'a mut [u8],
    ) -> Option<Result<ReadResult, FramerError<E>>>
    where
        E: Debug,
    {
        loop {
            if self.rx_buf_range.is_empty() {
                match stream.next().await {
                    Some(Ok(input)) => {
                        if rx_buf.len() < input.len() {
                            return Some(Err(FramerError::RxBufferTooSmall(input.len())));
                        }

                        rx_buf[..input.len()].copy_from_slice(&input);
                        self.rx_buf_range = 0..input.len();
                    }
                    Some(Err(e)) => {
                        return Some(Err(FramerError::Io(e)));
                    }
                    None => return None,
                }
            }

            let ws_result = match self.websocket.read(
                &rx_buf[self.rx_buf_range.start..self.rx_buf_range.end],
                frame_buf,
            ) {
                Ok(ws_result) => ws_result,
                Err(e) => return Some(Err(FramerError::WebSocket(e))),
            };

            self.rx_buf_range.start += ws_result.len_from;

            match ws_result.message_type {
                WebSocketReceiveMessageType::Binary => {
                    self.frame_cursor += ws_result.len_to;
                    if ws_result.end_of_message {
                        let range = 0..self.frame_cursor;
                        self.frame_cursor = 0;
                        return Some(Ok(ReadResult::Binary(range)));
                    }
                }
                WebSocketReceiveMessageType::Text => {
                    self.frame_cursor += ws_result.len_to;
                    if ws_result.end_of_message {
                        let range = 0..self.frame_cursor;
                        self.frame_cursor = 0;
                        return Some(Ok(ReadResult::Text(range)));
                    }
                }
                WebSocketReceiveMessageType::CloseMustReply => {
                    let range = self.frame_cursor..self.frame_cursor + ws_result.len_to;
                    match self.websocket.write(
                        WebSocketSendMessageType::CloseReply,
                        true,
                        &frame_buf[range.start..range.end],
                        tx_buf,
                    ) {
                        Ok(len) => match stream.send(&tx_buf[..len]).await {
                            Ok(()) => {
                                self.frame_cursor = 0;
                                return Some(Ok(ReadResult::Close(range)));
                            }
                            Err(e) => return Some(Err(FramerError::Io(e))),
                        },
                        Err(e) => return Some(Err(FramerError::WebSocket(e))),
                    }
                }
                WebSocketReceiveMessageType::CloseCompleted => return None,
                WebSocketReceiveMessageType::Pong => {
                    let range = self.frame_cursor..self.frame_cursor + ws_result.len_to;
                    return Some(Ok(ReadResult::Pong(range)));
                }
                WebSocketReceiveMessageType::Ping => {
                    let range = self.frame_cursor..self.frame_cursor + ws_result.len_to;
                    match self.websocket.write(
                        WebSocketSendMessageType::Pong,
                        true,
                        &frame_buf[range.start..range.end],
                        tx_buf,
                    ) {
                        Ok(len) => match stream.send(&tx_buf[..len]).await {
                            Ok(()) => {
                                return Some(Ok(ReadResult::Ping(range)));
                            }
                            Err(e) => return Some(Err(FramerError::Io(e))),
                        },
                        Err(e) => return Some(Err(FramerError::WebSocket(e))),
                    }
                }
            }

            return None;
        }
    }
}
