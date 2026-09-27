//! Bounded, cancellable HTTP operations. Never format reqwest errors (they contain URLs).
use crate::resource_limits::ResourceLimits;
use anyhow::{Result, bail, ensure};
use reqwest::{Client, RequestBuilder, Response};
use std::time::Duration;

pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
// Preserve the pre-hardening ten-second request budget. In reqwest the read
// timer starts before headers (including connection/TLS), so a shorter value
// silently cuts off otherwise valid sign-in/discovery/provisioning requests.
// The total deadline still includes all header and body time; reads do not
// restart it. Explicit phase limits must not shorten that established budget.
pub(crate) const CONNECT_TIMEOUT: Duration = REQUEST_TIMEOUT;
pub(crate) const READ_TIMEOUT: Duration = REQUEST_TIMEOUT;

pub(crate) fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| anyhow::anyhow!("could not initialize secure HTTP client"))
}

pub(crate) fn network_error(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        if error.is_connect() {
            anyhow::anyhow!("network connection timed out")
        } else {
            anyhow::anyhow!("network request timed out")
        }
    } else if error.is_connect() {
        anyhow::anyhow!("could not connect to service; check network connection")
    } else if error.is_decode() || error.is_body() {
        anyhow::anyhow!("malformed or interrupted response body")
    } else {
        anyhow::anyhow!("network request failed; retry the operation")
    }
}

pub(crate) async fn send(request: RequestBuilder) -> Result<Response> {
    request.send().await.map_err(network_error)
}

#[derive(Clone, Copy)]
pub(crate) enum Payload {
    Metadata,
    Image,
}

pub(crate) async fn body(
    mut response: Response,
    kind: Payload,
    limits: ResourceLimits,
) -> Result<Vec<u8>> {
    let limits = limits.validate()?;
    let limit = match kind {
        Payload::Metadata => limits.metadata_bytes,
        Payload::Image => limits.image_bytes,
    };
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "response too large"
    );
    if let Some(mime) = response.headers().get(reqwest::header::CONTENT_TYPE) {
        let mime = mime
            .to_str()
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let accepted = match kind {
            Payload::Metadata => mime == "application/json" || mime.ends_with("+json"),
            Payload::Image => matches!(
                mime.as_str(),
                "image/png" | "image/jpeg" | "application/octet-stream"
            ),
        };
        ensure!(accepted, "unsupported response media type");
    }
    // No compressed HTTP features are enabled. PNG/JPEG decompression has separate limits.
    let declared = response.content_length();
    tokio::time::timeout(REQUEST_TIMEOUT, async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout(READ_TIMEOUT, response.chunk())
            .await
            .map_err(|_| anyhow::anyhow!("response read timed out"))?
            .map_err(network_error)?
        {
            ensure!(
                chunk.len() <= limit.saturating_sub(bytes.len()),
                "response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        ensure!(
            declared.is_none_or(|n| n == bytes.len() as u64),
            "malformed response length"
        );
        Ok(bytes)
    })
    .await
    .map_err(|_| anyhow::anyhow!("response read timed out"))?
}

pub(crate) async fn json<T: serde::de::DeserializeOwned>(response: Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        bail!("service rejected request (HTTP {})", status.as_u16());
    }
    let bytes = body(response, Payload::Metadata, ResourceLimits::default()).await?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("malformed JSON response"))
}

pub(crate) fn regional_endpoint(host: &str) -> Result<reqwest::Url> {
    let url =
        reqwest::Url::parse(host).map_err(|_| anyhow::anyhow!("invalid streaming endpoint"))?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && matches!(url.path(), "" | "/")
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(".xboxlive.com")),
        "untrusted streaming endpoint"
    );
    Ok(url)
}

pub(crate) fn decode_image(bytes: &[u8], limits: ResourceLimits) -> Result<image::DynamicImage> {
    let limits = limits.validate()?;
    ensure!(
        bytes.len() <= limits.image_bytes,
        "image download exceeds limit"
    );
    let format =
        image::guess_format(bytes).map_err(|_| anyhow::anyhow!("malformed image payload"))?;
    ensure!(
        matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg),
        "unsupported image media type"
    );
    let make_reader = || image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = make_reader()
        .into_dimensions()
        .map_err(|_| anyhow::anyhow!("malformed image header"))?;
    limits.image_dimensions(width, height)?;
    let mut reader = make_reader();
    let mut decoder_limits = image::Limits::default();
    decoder_limits.max_image_width = Some(limits.image_width);
    decoder_limits.max_image_height = Some(limits.image_height);
    decoder_limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(decoder_limits);
    reader
        .decode()
        .map_err(|_| anyhow::anyhow!("image decode failed"))
}

pub(crate) fn api_url(host: &str, path: &str) -> Result<reqwest::Url> {
    let base = regional_endpoint(host)?;
    ensure!(
        path.starts_with('/')
            && !path.starts_with("//")
            && !path.contains(['\\', '#'])
            && !path.chars().any(char::is_control),
        "invalid API path"
    );
    let url = base
        .join(path)
        .map_err(|_| anyhow::anyhow!("invalid API path"))?;
    ensure!(
        url.origin() == base.origin() && url.username().is_empty() && url.password().is_none(),
        "untrusted API destination"
    );
    Ok(url)
}
