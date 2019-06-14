pub struct WebSocket<'a, T>
    where T: RngCore {
    is_client: bool,
    rng: &'a mut Option<T>,
    continuation_frame_op_code: Option<WebSocketOpCode>,
    state: WebSocketState,
    continuation_read: Option<ContinuationRead>,
}

impl<'a, T> WebSocket<'a, T>
    where
        T: RngCore,
{
    pub fn new_client(rng: &'a mut Option<T>) -> WebSocket<'a, T> {
        WebSocketClient {
            is_client: true,
            rng,
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }
/*
    pub fn new_server() -> WebSocket<T> + 'static {
        WebSocket {
            is_client: false,
            rng: None,
            continuation_frame_op_code: None,
            state: WebSocketState::None,
            continuation_read: None,
        }
    }*/
}