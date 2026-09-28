//! Actual Stream signaling methods with a recording HTTP boundary. No Xbox
//! server is emulated; these tests prove which requests the client issues.
#![allow(dead_code)]
extern crate self as rtc;
pub mod peer_connection {
    pub mod transport {
        #[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone)]
        pub struct RTCIceCandidateInit {
            pub candidate: String,
            pub sdp_mid: Option<String>,
            pub sdp_mline_index: Option<u16>,
            pub username_fragment: Option<String>,
            pub url: Option<String>,
        }
    }
}
#[path = "../../../src/api_xbox/chat_sdp.rs"]
pub mod chat_sdp;
#[path = "../../../src/api_xbox/session_kind.rs"]
pub mod session_kind;
#[path = "../../../src/api_xbox/stream.rs"]
pub mod stream;
pub use stream::Stream;
#[path = "../../../src/jobs.rs"]
pub mod jobs;
#[cfg(test)]
mod refresh;
#[cfg(test)]
pub use refresh::{api, streaming};
pub mod api_xbox {
    pub use crate::session_kind;
    #[cfg(test)]
    pub mod streaming {
        pub use crate::refresh::backend;
        pub mod rtc {
            pub use crate::refresh::worker;
        }
    }
    pub mod auth {
        #[derive(Debug, Clone)]
        pub struct EndpointCredentials;
        pub struct MsalAuth;
        impl MsalAuth {
            pub async fn get_passport_token(&mut self) -> anyhow::Result<String> {
                Ok(String::new())
            }
        }
    }
    pub mod api {
        #[derive(Debug, Clone, Default)]
        pub struct ApiClient(
            pub std::sync::Arc<std::sync::Mutex<Vec<(reqwest::Method, String)>>>,
            pub std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>>,
        );
        impl ApiClient {
            pub async fn request_json<T: serde::de::DeserializeOwned>(
                &self,
                _: &super::auth::EndpointCredentials,
                method: reqwest::Method,
                path: &str,
                _: Option<&serde_json::Value>,
            ) -> anyhow::Result<T> {
                self.0
                    .lock()
                    .unwrap()
                    .push((method.clone(), path.to_owned()));
                let value = if method == reqwest::Method::GET && path.ends_with("/state") {
                    self.1
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("scripted state response")
                } else if method == reqwest::Method::GET && path.ends_with("/sdp") {
                    serde_json::json!({"exchangeResponse": "{\"sdp\":\"v=0\\r\\n\"}"})
                } else {
                    serde_json::json!({})
                };
                Ok(serde_json::from_value(value)?)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn ready_to_connect_is_one_handshake_per_owned_session_across_mode_changes() {
        let api = api_xbox::api::ApiClient::default();
        // Actual Stream methods, scripted service boundary; no claim of Xbox auth.
        for (kind, name) in [
            (session_kind::StreamKind::Cloud, "cloud"),
            (session_kind::StreamKind::Home, "home"),
            (session_kind::StreamKind::Home, "home"),
        ] {
            let mut stream = Stream::new(
                api.clone(),
                api_xbox::auth::EndpointCredentials,
                stream::StartStreamResponse {
                    session_path: format!("/v5/sessions/{name}/owned"),
                },
                kind,
            );
            for state in [
                "Provisioning",
                "ReadyToConnect",
                "ReadyToConnect",
                "Provisioned",
            ] {
                api.1
                    .lock()
                    .unwrap()
                    .push_back(serde_json::json!({"state":state}));
                stream
                    .poll_provisioning(&mut api_xbox::auth::MsalAuth)
                    .await
                    .unwrap();
                // The app clones state into each background polling job.
                stream = stream.clone();
            }
            stream.send_sdp_offer("v=0\r\n").await.unwrap();
            stream.stop().await.unwrap();
        }
        let requests = api.0.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|(m, p)| *m == reqwest::Method::POST && p.ends_with("/connect"))
                .count(),
            3
        );
        assert_eq!(
            requests
                .iter()
                .filter(|(m, _)| *m == reqwest::Method::DELETE)
                .count(),
            1
        );
        assert!(
            requests
                .iter()
                .filter(|(m, _)| *m == reqwest::Method::DELETE)
                .all(|(_, p)| p.contains("/cloud/"))
        );
    }

    #[tokio::test]
    async fn failed_home_state_keeps_service_code_and_terminal_identity() {
        let api = api_xbox::api::ApiClient::default();
        api.1.lock().unwrap().push_back(serde_json::json!({"state":"Failed", "detailedSessionState":17,
            "errorDetails":{"code":"AgentCommandError","message":"private server text must not leak"}}));
        let mut stream = Stream::new(
            api.clone(),
            api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse {
                session_path: "/v5/sessions/home/owned".into(),
            },
            session_kind::StreamKind::Home,
        );
        let error = stream
            .poll_provisioning(&mut api_xbox::auth::MsalAuth)
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<stream::ProvisioningFailure>()
                .is_some()
        );
        let text = error.to_string();
        assert!(
            text.contains("AgentCommandError")
                && text.contains("detail=17")
                && text.contains("connect accepted=false")
        );
        assert!(!text.contains("private"));
        assert_eq!(api.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn repeated_media_sdp_exchange_uses_the_existing_session_endpoint_only() {
        let api = api_xbox::api::ApiClient::default();
        let stream = stream::Stream::new(
            api.clone(),
            api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse {
                session_path: "/v5/sessions/home/owned".into(),
            },
            session_kind::StreamKind::Home,
        );
        for _ in 0..100 {
            assert_eq!(stream.send_sdp_offer("v=0\r\n").await.unwrap(), "v=0\r\n");
        }
        let requests = api.0.lock().unwrap();
        assert_eq!(requests.len(), 200);
        for pair in requests.as_chunks::<2>().0 {
            assert_eq!(
                pair,
                &[
                    (reqwest::Method::POST, "/v5/sessions/home/owned/sdp".into()),
                    (reqwest::Method::GET, "/v5/sessions/home/owned/sdp".into()),
                ]
            );
        }
    }
    #[tokio::test]
    async fn home_cleanup_never_issues_a_termination_request() {
        let api = api_xbox::api::ApiClient::default();
        let stream = stream::Stream::new(
            api.clone(),
            api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse {
                session_path: "/v5/sessions/home/owned".into(),
            },
            session_kind::StreamKind::Home,
        );
        for _ in 0..100 {
            stream.clone().stop().await.unwrap();
        }
        assert!(api.0.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn cloud_cleanup_only_deletes_the_owned_session() {
        let api = api_xbox::api::ApiClient::default();
        let stream = stream::Stream::new(
            api.clone(),
            api_xbox::auth::EndpointCredentials,
            stream::StartStreamResponse {
                session_path: "/v5/sessions/cloud/owned".into(),
            },
            session_kind::StreamKind::Cloud,
        );
        stream.stop().await.unwrap();
        assert_eq!(
            *api.0.lock().unwrap(),
            vec![(reqwest::Method::DELETE, "/v5/sessions/cloud/owned".into())]
        );
    }
}
