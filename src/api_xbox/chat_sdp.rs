//! Xbox distinguishes chat activation from the initial streaming SDP exchange.
use serde_json::{Value, json};

pub(crate) fn offer_body(sdp: &str) -> Value {
    json!({
        "messageType": "offer",
        "requestId": 2,
        "sdp": sdp,
        "configuration": { "isMediaStreamsChatRenegotiation": true },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_offer_uses_xbox_chat_contract_without_restarting_stream_configuration() {
        let body = super::offer_body("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n");
        assert_eq!(body["messageType"], "offer");
        assert_eq!(body["requestId"], 2);
        assert_eq!(body["configuration"], serde_json::json!({"isMediaStreamsChatRenegotiation": true}));
        assert!(body["sdp"].as_str().unwrap().contains("m=audio"));
    }
}
