//! HTTP client for the xCloud/xHome streaming APIs: sessions, titles, consoles, wait times.

use crate::api_xbox::auth::EndpointCredentials;
use crate::api_xbox::stream::{StartStreamResponse, Stream};
use anyhow::{Context, Result};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde_json::{Value, json};

const DEVICE_INFO_JSON: &str = r#"{"appInfo":{"env":{"clientAppId":"www.xbox.com","clientAppType":"browser","clientAppVersion":"26.1.97","clientSdkVersion":"10.3.7","httpEnvironment":"prod","sdkInstallId":""}},"dev":{"hw":{"make":"Microsoft","model":"unknown","sdktype":"web"},"os":{"name":"android","ver":"22631.2715","platform":"desktop"},"displayInfo":{"dimensions":{"widthInPixels":960,"heightInPixels":540},"pixelDensity":{"dpiX":1,"dpiY":1}},"browser":{"browserName":"chrome","browserVersion":"140.0.3485.54"}}}"#;

#[derive(Debug, Clone)]
pub struct ApiClientConfig {
    pub locale: String,
    pub home: EndpointCredentials,
    pub cloud: EndpointCredentials,
    pub cloud_f2p: Option<EndpointCredentials>,
}

impl Default for ApiClientConfig {
    fn default() -> Self {
        Self {
            locale: "en-US".to_owned(),
            home: EndpointCredentials {
                host: String::new(),
                token: String::new(),
            },
            cloud: EndpointCredentials {
                host: String::new(),
                token: String::new(),
            },
            cloud_f2p: None,
        }
    }
}

pub use super::session_kind::StreamKind;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolesResponse {
    pub results: Vec<Console>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitTimeResponse {
    pub estimated_total_wait_time_in_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Console {
    pub device_name: String,
    pub server_id: String,
    pub power_state: String,
    pub console_type: String,
}

#[derive(Debug, Clone)]
pub struct ApiClient {
    client: Option<Client>,
    pub config: ApiClientConfig,
}

impl ApiClient {
    pub fn new(config: ApiClientConfig) -> Self {
        Self {
            client: crate::http::client().ok(),
            config,
        }
    }

    fn client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .context("secure HTTP client unavailable")
    }

    fn credentials_for(&self, kind: StreamKind) -> &EndpointCredentials {
        match kind {
            StreamKind::Home => &self.config.home,
            StreamKind::Cloud => &self.config.cloud,
        }
    }

    pub async fn get_consoles(&self) -> Result<ConsolesResponse> {
        self.get_json(StreamKind::Home, "/v6/servers/home")
            .await
            .context("console discovery failed")
    }

    pub async fn get_titles(&self) -> Result<Value> {
        self.get_json(StreamKind::Cloud, "/v2/titles").await
    }

    pub(crate) async fn get_recent_titles(&self, continuation: Option<&str>) -> Result<Value> {
        let mut path = String::from("/v2/titles/mru?mr=50");
        if let Some(token) = continuation {
            let mut url = reqwest::Url::parse("https://unused.invalid/v2/titles/mru?mr=50")?;
            url.query_pairs_mut().append_pair("ct", token);
            path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        }
        self.get_json(StreamKind::Cloud, &path).await
    }

    pub(crate) async fn get_gallery(
        &self,
        id: &str,
        market: &str,
        language: &str,
    ) -> Result<Value> {
        // Public editorial feed: never attach the account's streaming token.
        crate::http::json(
            crate::http::send(
                self.client()?
                    .get("https://catalog.gamepass.com/sigls/v2")
                    .query(&[("id", id), ("market", market), ("language", language)]),
            )
            .await?,
        )
        .await
    }

    pub async fn get_wait_time(
        &self,
        kind: StreamKind,
        target_id: &str,
    ) -> Result<WaitTimeResponse> {
        self.get_json(kind, &format!("/v1/waittime/{target_id}"))
            .await
    }

    /// Creates a streaming session. Cloud titles missing from the primary offering are retried
    /// against the free-to-play endpoint when one is available.
    pub async fn start_stream(&self, kind: StreamKind, title_or_server_id: &str) -> Result<Stream> {
        let body = json!({
            "clientSessionId": "",
            "titleId": if kind == StreamKind::Cloud { title_or_server_id } else { "" },
            "systemUpdateGroup": "",
            "settings": {
                "nanoVersion": "V3;WebrtcTransport.dll",
                "enableOptionalDataCollection": false,
                "enableTextToSpeech": false,
                "highContrast": 0,
                "locale": self.config.locale,
                "useIceConnection": false,
                "timezoneOffsetMinutes": 120,
                "sdkType": "web",
                "osName": "android",
            },
            "serverId": if kind == StreamKind::Home { title_or_server_id } else { "" },
            "fallbackRegionNames": [],
        });
        let path = format!("/v5/sessions/{}/play", kind.as_path());

        if kind == StreamKind::Home {
            return self
                .start_stream_with_credentials(kind, self.config.home.clone(), &path, &body)
                .await;
        }

        match self
            .start_stream_with_credentials(kind, self.config.cloud.clone(), &path, &body)
            .await
        {
            Ok(stream) => Ok(stream),
            Err(error) if error.to_string().contains("OfferingDoesNotContainTitle") => {
                let Some(fallback) = self.config.cloud_f2p.clone() else {
                    return Err(error);
                };
                self.start_stream_with_credentials(kind, fallback, &path, &body)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn start_stream_with_credentials(
        &self,
        kind: StreamKind,
        credentials: EndpointCredentials,
        path: &str,
        body: &Value,
    ) -> Result<Stream> {
        let response: StartStreamResponse = self
            .request_json(&credentials, Method::POST, path, Some(body))
            .await?;
        Ok(Stream::new(self.clone(), credentials, response, kind))
    }

    pub async fn get_json<T>(&self, kind: StreamKind, path: &str) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        self.request_json(self.credentials_for(kind), Method::GET, path, None)
            .await
    }

    /// Shared HTTP+JSON path for both `kind`-routed requests above and `Stream`'s per-session
    /// credential requests in `api_xbox::stream`.
    pub(super) async fn request_json<T>(
        &self,
        credentials: &EndpointCredentials,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = crate::http::api_url(&credentials.host, path)?;
        let mut request = self
            .client()?
            .request(method, url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("X-Gssv-Client", "XboxComBrowser")
            .header("X-MS-Device-Info", DEVICE_INFO_JSON);

        if !credentials.token.is_empty() {
            request = request.bearer_auth(&credentials.token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = crate::http::send(request)
            .await
            .context("waiting for service response")?;
        let status = response.status();
        let bytes = crate::http::body(
            response,
            crate::http::Payload::Metadata,
            crate::resource_limits::ResourceLimits::default(),
        )
        .await
        .context("reading service response")?;
        if !status.is_success() {
            // Preserve only this known protocol code; never echo arbitrary server text.
            let fallback = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .is_some_and(|value| {
                    value
                        .get("errorDetails")
                        .and_then(Value::as_str)
                        .is_some_and(|s| s.contains("OfferingDoesNotContainTitle"))
                        || value.get("code").and_then(Value::as_str)
                            == Some("OfferingDoesNotContainTitle")
                        || value.pointer("/errorDetails/code").and_then(Value::as_str)
                            == Some("OfferingDoesNotContainTitle")
                });
            if fallback {
                anyhow::bail!("OfferingDoesNotContainTitle");
            }
            anyhow::bail!("xCloud request rejected (HTTP {})", status.as_u16());
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return serde_json::from_value(json!({ "status": status.as_u16() }))
                .context("failed to decode empty-body status marker");
        }
        serde_json::from_slice::<T>(&bytes)
            .map_err(|_| anyhow::anyhow!("malformed xCloud JSON response"))
    }
}

#[cfg(test)]
mod tests {
    use super::{ConsolesResponse, StreamKind};

    #[test]
    fn home_console_response_preserves_server_id() {
        let response: ConsolesResponse = serde_json::from_str(
            r#"{"results":[{"deviceName":"Living room Xbox","serverId":"console-123","powerState":"On","consoleType":"XboxSeriesX"}]}"#,
        )
        .unwrap();
        assert_eq!(StreamKind::Home.as_path(), "home");
        assert_eq!(response.results[0].server_id, "console-123");
    }
}
