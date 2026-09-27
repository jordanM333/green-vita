use anyhow::{Context, Result, bail};
use reqwest::Client;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const CLIENT_ID: &str = "1f907974-e22b-4810-a9de-d9647380c97e";
const OAUTH_SCOPE: &str = "xboxlive.signin openid profile offline_access";
const TOKEN_STORE_DIR: &str = "ux0:data/green-vita-540-test";
const TOKEN_STORE_PATH: &str = "ux0:data/green-vita-540-test/xcloud-tokens.json";
const TOKEN_STORE_VERSION: u8 = 1;
const TOKEN_KEY_MAGIC: &[u8; 8] = b"GVTKEY01";
const TOKEN_KEY_SIZE: usize = 32;
const TOKEN_KEY_RECORD_SIZE: usize = TOKEN_KEY_MAGIC.len() + TOKEN_KEY_SIZE;
const TOKEN_KEY_OFFSET: i64 = 0;
const TOKEN_NONCE_SIZE: usize = 12;
const TOKEN_AAD: &[u8] = b"green-vita/xcloud-refresh-token/v1";

#[derive(Clone)]
pub struct EndpointCredentials {
    pub host: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct StreamingCredentials {
    pub home: EndpointCredentials,
    pub cloud: EndpointCredentials,
    pub cloud_f2p: Option<EndpointCredentials>,
}

#[derive(Clone, Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    message: String,
}

#[derive(Clone)]
pub struct DeviceCodeAuth {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
    device_code: String,
    pub poll_interval: Duration,
    deadline: Instant,
}

impl DeviceCodeAuth {
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

#[derive(Clone, Deserialize)]
struct UserTokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct DeviceCodeErrorResponse {
    error: String,
}

pub enum DeviceCodePoll {
    Pending(Duration),
    Authorized,
    Restart,
}

#[derive(Clone, Deserialize, Serialize)]
struct TokenStoreData {
    version: u8,
    nonce: String,
    ciphertext: String,
}

#[derive(Clone, Deserialize)]
struct LegacyTokenStoreData {
    refresh_token: String,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum StoredTokenData {
    Encrypted(TokenStoreData),
    Legacy(LegacyTokenStoreData),
}

#[derive(Clone, Deserialize)]
struct XstsTokenResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: XstsDisplayClaims,
}

#[derive(Clone, Deserialize)]
struct XstsDisplayClaims {
    xui: Vec<XstsUserClaim>,
}

#[derive(Clone, Deserialize)]
struct XstsUserClaim {
    uhs: String,
}

pub struct XboxProfile {
    pub gamertag: Option<String>,
    pub gamerscore: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Deserialize)]
struct ProfileResponse {
    #[serde(rename = "profileUsers")]
    profile_users: Vec<ProfileUser>,
}

#[derive(Clone, Deserialize)]
struct ProfileUser {
    settings: Vec<ProfileSetting>,
}

#[derive(Clone, Deserialize)]
struct ProfileSetting {
    id: String,
    value: String,
}

#[derive(Clone, Deserialize)]
struct StreamingTokenResponse {
    #[serde(rename = "gsToken")]
    gs_token: String,
    #[serde(rename = "offeringSettings")]
    offering_settings: OfferingSettings,
}

#[derive(Clone, Deserialize)]
struct OfferingSettings {
    regions: Vec<StreamingRegion>,
}

#[derive(Clone, Deserialize)]
struct StreamingRegion {
    #[serde(rename = "baseUri")]
    base_uri: String,
    #[serde(rename = "isDefault")]
    is_default: bool,
}

impl StreamingTokenResponse {
    fn into_credentials(self) -> Result<EndpointCredentials> {
        let region = self
            .offering_settings
            .regions
            .into_iter()
            .find(|region| region.is_default)
            .context("streaming token response had no default region")?;

        Ok(EndpointCredentials {
            host: region.base_uri,
            token: self.gs_token,
        })
    }
}

#[derive(Clone)]
pub struct MsalAuth {
    client: Option<Client>,
    refresh_token: Option<String>,
    storage_error: Option<String>,
}

impl Default for MsalAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl MsalAuth {
    pub fn new() -> Self {
        let _ = ensure_token_store_dir();
        let mut storage_error = None;
        let refresh_token = match load_saved_refresh_token() {
            Ok(Some(SavedRefreshToken::Encrypted(token))) => Some(token),
            Ok(Some(SavedRefreshToken::Legacy(token))) => {
                match migrate_legacy_token(token, persist_refresh_token, || {
                    clear_saved_login_at(std::path::Path::new(TOKEN_STORE_PATH), clear_token_key)
                }) {
                    Ok(token) => Some(token),
                    Err(error) => {
                        storage_error = Some(error.to_string());
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(_) => {
                storage_error = Some(
                    "Saved login could not be read. Sign out to clear it, then sign in again."
                        .into(),
                );
                None
            }
        };

        Self {
            client: crate::http::client().ok(),
            refresh_token,
            storage_error,
        }
    }

    fn client(&self) -> Result<&Client> {
        self.client
            .as_ref()
            .context("secure HTTP client unavailable")
    }

    pub fn has_saved_login(&self) -> bool {
        self.refresh_token.is_some()
    }

    pub fn take_storage_error(&mut self) -> Option<String> {
        self.storage_error.take()
    }

    pub fn logout(&mut self) -> Result<()> {
        self.storage_error = None;
        self.clear_saved_login()
    }

    fn save_refresh_token(&mut self, refresh_token: String) {
        self.refresh_token = Some(refresh_token.clone());

        let result = persist_refresh_token(&refresh_token);
        if let Err(error) = result {
            eprintln!("Could not persist xCloud login: {error:#}");
        }
    }

    fn clear_saved_login(&mut self) -> Result<()> {
        self.refresh_token = None;
        clear_saved_login_at(std::path::Path::new(TOKEN_STORE_PATH), clear_token_key)
    }

    async fn post_form(
        &self,
        url: &str,
        body: String,
        context_label: &str,
    ) -> Result<reqwest::Response> {
        self.client()?
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(crate::http::network_error)
            .with_context(|| format!("{context_label} failed"))
    }

    pub async fn request_device_code(&self) -> Result<DeviceCodeAuth> {
        let response: DeviceCodeResponse = crate::http::json(
            self.post_form(
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode",
                format!("client_id={CLIENT_ID}&scope={}", urlencode(OAUTH_SCOPE)),
                "device code request",
            )
            .await?,
        )
        .await?;
        anyhow::ensure!(
            (1..=3600).contains(&response.expires_in) && (1..=60).contains(&response.interval),
            "invalid device authorization timing"
        );
        let verification =
            reqwest::Url::parse(&response.verification_uri).context("invalid verification URL")?;
        anyhow::ensure!(
            verification.scheme() == "https"
                && verification.host_str().is_some_and(|h| h == "microsoft.com"
                    || h.ends_with(".microsoft.com")
                    || h == "aka.ms"),
            "untrusted verification URL"
        );

        Ok(DeviceCodeAuth {
            user_code: response.user_code,
            verification_uri: response.verification_uri,
            message: response.message,
            device_code: response.device_code,
            poll_interval: Duration::from_secs(response.interval.max(1)),
            deadline: Instant::now()
                .checked_add(Duration::from_secs(response.expires_in))
                .context("invalid authorization deadline")?,
        })
    }

    pub async fn poll_device_code(&mut self, auth: &DeviceCodeAuth) -> Result<DeviceCodePoll> {
        let response = self
            .post_form(
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
                format!(
                    "grant_type=urn:ietf:params:oauth:grant-type:device_code&client_id={CLIENT_ID}&device_code={}",
                    urlencode(&auth.device_code)
                ),
                "device code poll request",
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body =
                crate::http::body(response, crate::http::Payload::Metadata, Default::default())
                    .await?;
            let error: DeviceCodeErrorResponse = serde_json::from_slice(&body).map_err(|_| {
                anyhow::anyhow!(
                    "malformed authorization response (HTTP {})",
                    status.as_u16()
                )
            })?;

            return match error.error.as_str() {
                "authorization_pending" => Ok(DeviceCodePoll::Pending(auth.poll_interval)),
                "slow_down" => Ok(DeviceCodePoll::Pending(
                    (auth.poll_interval + Duration::from_secs(5)).min(Duration::from_secs(60)),
                )),
                "expired_token" | "bad_verification_code" => Ok(DeviceCodePoll::Restart),
                _ => bail!("device authorization rejected; please sign in again"),
            };
        }

        let token: UserTokenResponse = crate::http::json(response).await?;
        self.save_refresh_token(token.refresh_token);
        Ok(DeviceCodePoll::Authorized)
    }

    async fn refresh_user_token(&mut self) -> Result<String> {
        let Some(refresh_token) = self.refresh_token.clone() else {
            bail!("no saved xCloud login to refresh");
        };

        let response = self
            .post_form(
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
                format!(
                    "client_id={CLIENT_ID}&grant_type=refresh_token&refresh_token={}&scope={}",
                    urlencode(&refresh_token),
                    urlencode(OAUTH_SCOPE)
                ),
                "token refresh request",
            )
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body =
                crate::http::body(response, crate::http::Payload::Metadata, Default::default())
                    .await?;
            let oauth_error = serde_json::from_slice::<DeviceCodeErrorResponse>(&body).ok();

            if oauth_error
                .as_ref()
                .is_some_and(|error| error.error == "invalid_grant")
            {
                self.clear_saved_login()?;
                bail!("saved xCloud login expired; please sign in again");
            }
            bail!(
                "token refresh rejected (HTTP {}); please sign in again",
                status.as_u16()
            );
        }

        let token: UserTokenResponse = crate::http::json(response).await?;

        self.save_refresh_token(token.refresh_token.clone());
        Ok(token.access_token)
    }

    pub async fn get_passport_token(&mut self) -> Result<String> {
        self.refresh_user_token().await?;
        let Some(refresh_token) = self.refresh_token.clone() else {
            bail!("no saved xCloud login to derive a passport token from");
        };

        let response = self
            .post_form(
                "https://login.live.com/oauth20_token.srf",
                format!(
                    "client_id={CLIENT_ID}&scope=service::http://Passport.NET/purpose::PURPOSE_XBOX_CLOUD_CONSOLE_TRANSFER_TOKEN&grant_type=refresh_token&refresh_token={}", urlencode(&refresh_token)
                ),
                "passport token request",
            )
            .await?
            ;

        let token: UserTokenResponse = crate::http::json(response).await?;

        Ok(token.access_token)
    }

    /// Shared shape of every xCloud/Xbox Live token exchange below.
    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        url: impl reqwest::IntoUrl,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
        context_label: &str,
    ) -> Result<T> {
        let mut request = self.client()?.post(url).json(body);
        for (key, value) in headers {
            request = request.header(*key, *value);
        }
        let response = request
            .send()
            .await
            .map_err(crate::http::network_error)
            .with_context(|| format!("{context_label} request failed"))?;
        crate::http::json(response).await
    }

    async fn xsts_user_authenticate(&self, access_token: &str) -> Result<String> {
        let body = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "RpsTicket": format!("d={access_token}"),
                "SiteName": "user.auth.xboxlive.com",
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });

        let response: XstsTokenResponse = self
            .post_json(
                "https://user.auth.xboxlive.com/user/authenticate",
                &[("x-xbl-contract-version", "1")],
                &body,
                "XSTS user authentication",
            )
            .await?;

        Ok(response.token)
    }

    async fn xsts_authorize(
        &self,
        user_token: &str,
        relying_party: &str,
    ) -> Result<XstsTokenResponse> {
        let body = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [user_token],
            },
            "RelyingParty": relying_party,
            "TokenType": "JWT",
        });

        self.post_json(
            "https://xsts.auth.xboxlive.com/xsts/authorize",
            &[("x-xbl-contract-version", "1")],
            &body,
            "XSTS authorize",
        )
        .await
    }

    async fn streaming_token(
        &self,
        gssv_token: &str,
        offering: &str,
    ) -> Result<EndpointCredentials> {
        let body = serde_json::json!({
            "token": gssv_token,
            "offeringId": offering,
        });

        let response: StreamingTokenResponse = self
            .post_json(
                format!("https://{offering}.gssv-play-prod.xboxlive.com/v2/login/user"),
                &[("x-gssv-client", "XboxComBrowser")],
                &body,
                &format!("streaming token (offering {offering})"),
            )
            .await?;

        response.into_credentials()
    }

    pub async fn fetch_streaming_credentials(&mut self) -> Result<StreamingCredentials> {
        let access_token = self.refresh_user_token().await?;
        let web_token = self.xsts_user_authenticate(&access_token).await?;
        let gssv_token = self
            .xsts_authorize(&web_token, "http://gssv.xboxlive.com/")
            .await?
            .token;

        let home = self.streaming_token(&gssv_token, "xhome").await?;

        let primary = self.streaming_token(&gssv_token, "xgpuweb").await;
        let f2p = self.streaming_token(&gssv_token, "xgpuwebf2p").await;

        let (cloud, cloud_f2p) = match (primary, f2p) {
            (Ok(cloud), f2p) => (cloud, f2p.ok()),
            (Err(_), Ok(f2p)) => (f2p, None),
            (Err(error), Err(_)) => return Err(error),
        };

        Ok(StreamingCredentials {
            home,
            cloud,
            cloud_f2p,
        })
    }

    /// Gamertag + avatar URL, via a separate XSTS authorization for Xbox Live's own profile API.
    pub async fn fetch_xbox_profile(&mut self) -> Result<XboxProfile> {
        let access_token = self.refresh_user_token().await?;
        let web_token = self.xsts_user_authenticate(&access_token).await?;
        let xbl = self
            .xsts_authorize(&web_token, "http://xboxlive.com")
            .await?;
        // The "user hash" half of an `XBL3.0 x=<uhs>;<token>` Authorization header.
        let uhs = xbl
            .display_claims
            .xui
            .first()
            .map(|xui| xui.uhs.as_str())
            .context("XSTS response had no user hash (uhs)")?;

        let response: ProfileResponse = crate::http::json(self
            .client()?
            .get(
                "https://profile.xboxlive.com/users/me/profile/settings?settings=GameDisplayPicRaw,Gamertag,Gamerscore",
            )
            .header("x-xbl-contract-version", "3")
            .header("Authorization", format!("XBL3.0 x={uhs};{}", xbl.token))
            .send()
            .await
            .map_err(crate::http::network_error)?).await?;

        let settings = response
            .profile_users
            .into_iter()
            .next()
            .map(|user| user.settings)
            .unwrap_or_default();
        let setting = |id: &str| {
            settings
                .iter()
                .find(|setting| setting.id == id)
                .map(|setting| setting.value.clone())
        };

        Ok(XboxProfile {
            gamertag: setting("Gamertag"),
            gamerscore: setting("Gamerscore"),
            avatar_url: setting("GameDisplayPicRaw"),
        })
    }
}

fn ensure_token_store_dir() -> Result<()> {
    std::fs::create_dir_all(TOKEN_STORE_DIR).context("failed to create xCloud token directory")
}

enum SavedRefreshToken {
    Encrypted(String),
    Legacy(String),
}

fn load_saved_refresh_token() -> Result<Option<SavedRefreshToken>> {
    let file = match std::fs::File::open(TOKEN_STORE_PATH) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to open xCloud token store"),
    };
    match serde_json::from_slice::<StoredTokenData>(&{
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 65536, "saved token exceeds size limit");
        bytes
    })
    .map_err(|_| anyhow::anyhow!("malformed saved login data"))?
    {
        StoredTokenData::Encrypted(data) => decrypt_refresh_token(&data)
            .map(SavedRefreshToken::Encrypted)
            .map(Some),
        StoredTokenData::Legacy(data) => Ok(Some(SavedRefreshToken::Legacy(data.refresh_token))),
    }
}

// A legacy credential is usable only after an encrypted replacement succeeds.
fn migrate_legacy_token(
    token: String,
    persist: impl FnOnce(&str) -> Result<()>,
    clear: impl FnOnce() -> Result<()>,
) -> Result<String> {
    if persist(&token).is_ok() {
        return Ok(token);
    }
    clear().map_err(|_| anyhow::anyhow!("Could not secure or remove saved login. Check storage access and sign out before sharing this device."))?;
    bail!("Old saved login was removed because encryption failed. Sign in again.")
}

fn clear_saved_login_at(
    path: &std::path::Path,
    clear_key: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let removed = match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    // Always try both independent cleanup operations, even if the first fails.
    let key_cleared = clear_key().is_ok();
    anyhow::ensure!(
        removed && key_cleared,
        "Saved login cleanup failed. Check storage access and retry sign-out before sharing this device."
    );
    Ok(())
}

fn persist_refresh_token(refresh_token: &str) -> Result<()> {
    let data = encrypt_refresh_token(refresh_token)?;
    ensure_token_store_dir()?;
    crate::fs_utils::write_file_truncating(TOKEN_STORE_PATH, serde_json::to_string_pretty(&data)?)
        .context("failed to persist encrypted xCloud login token")
}

fn encrypt_refresh_token(refresh_token: &str) -> Result<TokenStoreData> {
    let key = load_or_create_token_key()?;
    let mut nonce = [0u8; TOKEN_NONCE_SIZE];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("failed to generate xCloud token nonce"))?;
    let cipher = token_cipher(&key)?;
    let mut ciphertext = refresh_token.as_bytes().to_vec();
    cipher
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(TOKEN_AAD),
            &mut ciphertext,
        )
        .map_err(|_| anyhow::anyhow!("failed to encrypt xCloud login token"))?;
    Ok(TokenStoreData {
        version: TOKEN_STORE_VERSION,
        nonce: encode_hex(&nonce),
        ciphertext: encode_hex(&ciphertext),
    })
}

fn decrypt_refresh_token(data: &TokenStoreData) -> Result<String> {
    if data.version != TOKEN_STORE_VERSION {
        bail!("unsupported xCloud token store version {}", data.version);
    }
    let nonce = decode_hex(&data.nonce).context("invalid xCloud token nonce")?;
    let nonce: [u8; TOKEN_NONCE_SIZE] = nonce
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid xCloud token nonce length"))?;
    let mut ciphertext = decode_hex(&data.ciphertext).context("invalid xCloud ciphertext")?;
    let key = load_token_key()?;
    let cipher = token_cipher(&key)?;
    let plaintext = cipher
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(TOKEN_AAD),
            &mut ciphertext,
        )
        .map_err(|_| anyhow::anyhow!("xCloud token authentication failed"))?;
    String::from_utf8(plaintext.to_vec()).context("decrypted xCloud token is not UTF-8")
}

fn token_cipher(key: &[u8; TOKEN_KEY_SIZE]) -> Result<LessSafeKey> {
    let key = UnboundKey::new(&aead::CHACHA20_POLY1305, key)
        .map_err(|_| anyhow::anyhow!("failed to initialize xCloud token cipher"))?;
    Ok(LessSafeKey::new(key))
}

fn load_token_key() -> Result<[u8; TOKEN_KEY_SIZE]> {
    let record = crate::safe_memory::load::<TOKEN_KEY_RECORD_SIZE>(TOKEN_KEY_OFFSET)?;
    if &record[..TOKEN_KEY_MAGIC.len()] != TOKEN_KEY_MAGIC {
        bail!("xCloud token key is missing from Safe Memory");
    }
    let mut key = [0u8; TOKEN_KEY_SIZE];
    key.copy_from_slice(&record[TOKEN_KEY_MAGIC.len()..]);
    Ok(key)
}

fn load_or_create_token_key() -> Result<[u8; TOKEN_KEY_SIZE]> {
    let existing = crate::safe_memory::load::<TOKEN_KEY_RECORD_SIZE>(TOKEN_KEY_OFFSET)?;
    if &existing[..TOKEN_KEY_MAGIC.len()] == TOKEN_KEY_MAGIC {
        let mut key = [0u8; TOKEN_KEY_SIZE];
        key.copy_from_slice(&existing[TOKEN_KEY_MAGIC.len()..]);
        return Ok(key);
    }
    anyhow::ensure!(
        existing.iter().all(|byte| *byte == 0),
        "unrecognized token key record; preserved without replacement"
    );
    let mut key = [0u8; TOKEN_KEY_SIZE];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| anyhow::anyhow!("failed to generate xCloud token key"))?;
    let mut record = [0u8; TOKEN_KEY_RECORD_SIZE];
    record[..TOKEN_KEY_MAGIC.len()].copy_from_slice(TOKEN_KEY_MAGIC);
    record[TOKEN_KEY_MAGIC.len()..].copy_from_slice(&key);
    crate::safe_memory::save(TOKEN_KEY_OFFSET, &record)?;
    Ok(key)
}

fn clear_token_key() -> Result<()> {
    crate::safe_memory::save(TOKEN_KEY_OFFSET, &[0u8; TOKEN_KEY_RECORD_SIZE])
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        bail!("hex value has an odd length");
    }
    encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = decode_hex_digit(pair[0])?;
            let low = decode_hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn decode_hex_digit(digit: u8) -> Result<u8> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        _ => bail!("invalid hex digit"),
    }
}

fn urlencode(value: &str) -> String {
    reqwest::Url::parse_with_params("https://form.invalid/", &[("v", value)])
        .ok()
        .and_then(|url| {
            url.query()
                .map(|query| query.trim_start_matches("v=").to_owned())
        })
        .unwrap_or_default()
}

impl std::fmt::Debug for EndpointCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EndpointCredentials { [redacted] }")
    }
}

impl std::fmt::Debug for DeviceCodeAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DeviceCodeAuth([redacted])")
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    #[test]
    fn legacy_migration_fails_closed_and_logout_attempts_both_resources() {
        use std::cell::Cell;
        let cleared = Cell::new(false);
        let result = migrate_legacy_token(
            "private-token".into(),
            |_| anyhow::bail!("private-endpoint"),
            || {
                cleared.set(true);
                Ok(())
            },
        );
        assert!(cleared.get());
        assert!(!result.unwrap_err().to_string().contains("private-"));
        let result = migrate_legacy_token(
            "private-token".into(),
            |_| anyhow::bail!("encryption"),
            || anyhow::bail!("unlink"),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Could not secure or remove")
        );
        assert_eq!(
            migrate_legacy_token(
                "token".into(),
                |_| Ok(()),
                || panic!("successful migration must not erase login")
            )
            .unwrap(),
            "token"
        );
        let folder = std::env::temp_dir().join(format!("greenvita-logout-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let key_attempted = Cell::new(false);
        assert!(
            clear_saved_login_at(&folder, || {
                key_attempted.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(key_attempted.get());
        let token = folder.join("token");
        std::fs::write(&token, b"secret").unwrap();
        assert!(clear_saved_login_at(&token, || anyhow::bail!("key cleanup")).is_err());
        assert!(!token.exists());
        assert!(clear_saved_login_at(&token, || Ok(())).is_ok());
    }

    #[test]
    fn form_encoding_and_secret_debug_do_not_expose_tokens() {
        assert_eq!(urlencode("a&b=c+%"), "a%26b%3Dc%2B%25");
        let credentials = EndpointCredentials {
            host: "private-endpoint".into(),
            token: "secret-token".into(),
        };
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("private-endpoint") && !debug.contains("secret-token"));
        assert!(decode_hex("00g1").is_err());
        assert!(decode_hex("0").is_err());
        assert_eq!(
            decode_hex(&encode_hex(&[0, 128, 255])).unwrap(),
            [0, 128, 255]
        );
    }
}
