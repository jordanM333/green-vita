//! Actual Stream signaling methods with a recording HTTP boundary. No Xbox
//! server is emulated; these tests prove which requests the client issues.
#![allow(dead_code)]
extern crate self as rtc;
pub mod peer_connection { pub mod transport {
    #[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone)]
    pub struct RTCIceCandidateInit {
        pub candidate: String, pub sdp_mid: Option<String>,
        pub sdp_mline_index: Option<u16>, pub username_fragment: Option<String>, pub url: Option<String>,
    }
} }
#[path = "../../../src/api_xbox/session_kind.rs"]
pub mod session_kind;
#[path = "../../../src/api_xbox/chat_sdp.rs"]
pub mod chat_sdp;
#[path = "../../../src/api_xbox/stream.rs"]
pub mod stream;
pub mod api_xbox {
    pub mod auth {
        #[derive(Debug, Clone)]
        pub struct EndpointCredentials;
        pub struct MsalAuth;
        impl MsalAuth { pub async fn get_passport_token(&mut self) -> anyhow::Result<String> { Ok(String::new()) } }
    }
    pub mod api {
        #[derive(Debug, Clone, Default)]
        pub struct ApiClient(pub std::sync::Arc<std::sync::Mutex<Vec<(reqwest::Method, String)>>>);
        impl ApiClient {
            pub async fn request_json<T: serde::de::DeserializeOwned>(&self, _: &super::auth::EndpointCredentials,
                method: reqwest::Method, path: &str, _: Option<&serde_json::Value>) -> anyhow::Result<T> {
                self.0.lock().unwrap().push((method, path.to_owned()));
                Ok(serde_json::from_value(serde_json::json!({}))?)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn home_cleanup_never_issues_a_termination_request() {
        let api = api_xbox::api::ApiClient::default();
        let stream = stream::Stream::new(api.clone(), api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse { session_path: "/v5/sessions/home/owned".into() }, session_kind::StreamKind::Home);
        for _ in 0..100 { stream.clone().stop().await.unwrap(); }
        assert!(api.0.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn cloud_cleanup_only_deletes_the_owned_session() {
        let api = api_xbox::api::ApiClient::default();
        let stream = stream::Stream::new(api.clone(), api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse { session_path: "/v5/sessions/cloud/owned".into() }, session_kind::StreamKind::Cloud);
        stream.stop().await.unwrap();
        assert_eq!(*api.0.lock().unwrap(), vec![(reqwest::Method::DELETE, "/v5/sessions/cloud/owned".into())]);
    }
}
