/*
pub fn client_initiate_opening_handshake(
    rng_bytes: fn(&mut bytes),
    websocket_options : &WebSocketOptions,
    to: &mut [u8],
) -> Result<(usize, WebSocketKey)> {
    let mut key: [u8; 16] = [0; 16];
    rng_bytes(&mut key);
    let mut key_as_base64: [u8; 24] = [0; 24];
    base64::encode(&key, &mut key_as_base64);
    let mut http_request: String<U1024> = String::new();
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
        for (i, sub_protocol) in sub_protocols.iter().enumerate(){
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
    Ok((http_request.len(), sec_websocket_key))
}

pub fn client_complete_opening_handshake(
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
            _ => return Err(Error::HttpResponseCodeInvalid)
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

        Ok(sec_websocket_protocol)
    }
    else {
        Err(Error::HttpHeaderIncomplete)
    }
}

pub fn server_respond_to_opening_handshake(
    sec_websocket_key: &WebSocketKey,
    sec_websocket_protocol: Option<&WebSocketSubProtocol>,
    to: &mut [u8],
) -> Result<usize> {

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
    Ok(http_response.len())
}
*/