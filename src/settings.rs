//! User-editable settings, persisted to `ux0:data/green-vita-540-test/settings.json` on each change.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const SETTINGS_DIR: &str = "ux0:data/green-vita-540-test";
const SETTINGS_PATH: &str = "ux0:data/green-vita-540-test/settings.json";
const MAX_SETTINGS_BYTES: usize = 1024 * 1024;
const MAX_SAVED_IDS: usize = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    EnUs,
    EnGb,
    PtBr,
    PtPt,
    EsEs,
    EsMx,
    FrFr,
    DeDe,
    ItIt,
    JaJp,
    KoKr,
    ZhCn,
    ZhTw,
    RuRu,
    PlPl,
    NlNl,
    SvSe,
    TrTr,
    ArSa,
}

impl Locale {
    pub const ALL: [Locale; 19] = [
        Self::EnUs,
        Self::EnGb,
        Self::PtBr,
        Self::PtPt,
        Self::EsEs,
        Self::EsMx,
        Self::FrFr,
        Self::DeDe,
        Self::ItIt,
        Self::JaJp,
        Self::KoKr,
        Self::ZhCn,
        Self::ZhTw,
        Self::RuRu,
        Self::PlPl,
        Self::NlNl,
        Self::SvSe,
        Self::TrTr,
        Self::ArSa,
    ];

    /// `(locale code, store market, native-language label)`.
    fn info(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::EnUs => ("en-US", "US", "English (US)"),
            Self::EnGb => ("en-GB", "GB", "English (UK)"),
            Self::PtBr => ("pt-BR", "BR", "Português (Brasil)"),
            Self::PtPt => ("pt-PT", "PT", "Português (Portugal)"),
            Self::EsEs => ("es-ES", "ES", "Español (España)"),
            Self::EsMx => ("es-MX", "MX", "Español (México)"),
            Self::FrFr => ("fr-FR", "FR", "Français"),
            Self::DeDe => ("de-DE", "DE", "Deutsch"),
            Self::ItIt => ("it-IT", "IT", "Italiano"),
            Self::JaJp => ("ja-JP", "JP", "日本語"),
            Self::KoKr => ("ko-KR", "KR", "한국어"),
            Self::ZhCn => ("zh-CN", "CN", "中文（简体）"),
            Self::ZhTw => ("zh-TW", "TW", "中文（繁體）"),
            Self::RuRu => ("ru-RU", "RU", "Русский"),
            Self::PlPl => ("pl-PL", "PL", "Polski"),
            Self::NlNl => ("nl-NL", "NL", "Nederlands"),
            Self::SvSe => ("sv-SE", "SE", "Svenska"),
            Self::TrTr => ("tr-TR", "TR", "Türkçe"),
            Self::ArSa => ("ar-SA", "SA", "العربية"),
        }
    }

    /// The locale code sent to xCloud, e.g. `"pt-BR"`.
    pub fn as_str(self) -> &'static str {
        self.info().0
    }

    /// The Microsoft Store catalog's `market` query param, e.g. `"BR"`.
    pub fn market(self) -> &'static str {
        self.info().1
    }

    /// Display label in the language's own native name, e.g. `"Русский"`.
    pub fn label(self) -> &'static str {
        self.info().2
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip)]
    pub persistence_error: bool,
    pub locale: Locale,
    /// Shows internal stream/session state on the `Streaming` screen. Off by default.
    pub show_stream_debug_info: bool,
    /// Playback volume of the mixed game/chat feed. Does not affect microphone input.
    pub stream_volume_percent: u8,
    /// Globally swaps only the rear L2/L3 and R2/R3 zones; front touch is unchanged.
    pub swap_rear_touch_trigger_stick: bool,
    /// Capture from the Vita's raw audio-in port instead of the voice port.
    /// The Vita still chooses the built-in or headset microphone itself.
    pub microphone_raw_input: bool,
    pub catalog: crate::catalog_preferences::CatalogPreferences,
    pub game_profiles: HashMap<String, GameProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GameProfile {
    pub swap_shoulders_and_triggers: bool,
    /// When enabled, front touch mirrors the rear L2/R2/L3/R3 zones instead of acting as a
    /// clickable xCloud pointer. Defaults to pointer mode for existing profiles.
    pub front_touch_auxiliary_buttons: bool,
    /// Controls only input from the physical rear touch panel. Front touch remains independent.
    pub rear_touch_enabled: bool,
}

impl Default for GameProfile {
    fn default() -> Self {
        Self {
            swap_shoulders_and_triggers: false,
            front_touch_auxiliary_buttons: false,
            rear_touch_enabled: true,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            persistence_error: false,
            locale: Locale::default(),
            show_stream_debug_info: false,
            stream_volume_percent: 100,
            swap_rear_touch_trigger_stick: false,
            microphone_raw_input: false,
            game_profiles: HashMap::new(),
            catalog: Default::default(),
        }
    }
}

impl Settings {
    pub fn game_profile(&self, title_id: &str) -> Option<&GameProfile> {
        self.game_profiles.get(title_id)
    }

    pub fn set_swap_shoulders_and_triggers(&mut self, title_id: String, enabled: bool) {
        self.game_profiles
            .entry(title_id)
            .or_default()
            .swap_shoulders_and_triggers = enabled;
    }

    pub fn set_front_touch_auxiliary_buttons(&mut self, title_id: String, enabled: bool) {
        self.game_profiles
            .entry(title_id)
            .or_default()
            .front_touch_auxiliary_buttons = enabled;
    }

    pub fn set_rear_touch_enabled(&mut self, title_id: String, enabled: bool) {
        self.game_profiles
            .entry(title_id)
            .or_default()
            .rear_touch_enabled = enabled;
    }

    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.stream_volume_percent <= 100, "invalid saved volume");
        anyhow::ensure!(
            self.game_profiles.len() <= MAX_SAVED_IDS
                && self.catalog.favorites.len() <= MAX_SAVED_IDS
                && self.catalog.recently_played.len() <= MAX_SAVED_IDS,
            "too many saved game IDs"
        );
        for id in self
            .game_profiles
            .keys()
            .chain(self.catalog.favorites.iter())
            .chain(self.catalog.recently_played.iter())
        {
            anyhow::ensure!(
                !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
                "invalid saved game ID"
            );
        }
        Ok(())
    }

    fn load_path(path: &str) -> anyhow::Result<Self> {
        let data = crate::fs_utils::read_bounded(path, MAX_SETTINGS_BYTES)?;
        let settings: Self = serde_json::from_slice(&data).context("invalid saved settings")?;
        settings.validate()?;
        Ok(settings)
    }

    /// Invalid originals remain on disk. Saving will refuse to replace them.
    pub fn load() -> Self {
        match Self::load_path(SETTINGS_PATH) {
            Ok(settings) => settings,
            Err(_) => {
                let missing = std::fs::metadata(SETTINGS_PATH)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
                Self {
                    persistence_error: !missing,
                    ..Self::default()
                }
            }
        }
    }

    fn save_path(&self, path: &str) -> anyhow::Result<()> {
        self.validate()?;
        match std::fs::metadata(path) {
            Ok(_) => {
                Self::load_path(path)
                    .context("existing settings require recovery; original preserved")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => anyhow::bail!("cannot inspect existing settings; original preserved"),
        }
        let data = serde_json::to_vec_pretty(self).context("could not serialize settings")?;
        anyhow::ensure!(
            data.len() <= MAX_SETTINGS_BYTES,
            "settings exceed size limit"
        );
        crate::fs_utils::write_file_truncating(path, data)
    }

    /// A visible localized banner reports failure without discarding current edits.
    pub fn save(&mut self) {
        self.persistence_error = std::fs::create_dir_all(SETTINGS_DIR)
            .context("could not create settings directory")
            .and_then(|_| self.save_path(SETTINGS_PATH))
            .is_err();
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn validation_preserves_original_corrupt_file() {
        let path =
            std::env::temp_dir().join(format!("greenvita-corrupt-{}.json", std::process::id()));
        std::fs::write(&path, b"{broken").unwrap();
        assert!(
            Settings::default()
                .save_path(path.to_str().unwrap())
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        let bad: Settings = serde_json::from_str(r#"{"stream_volume_percent":255}"#).unwrap();
        assert!(bad.validate().is_err());
        assert!(Settings::default().validate().is_ok());
    }

    #[test]
    fn older_settings_keep_profiles_and_default_to_original_rear_layout() {
        let saved = r#"{"show_stream_debug_info":true,"game_profiles":{"game-id":{"rear_touch_enabled":false}}}"#;
        let settings: Settings = serde_json::from_str(saved).unwrap();
        assert!(settings.catalog.favorites.is_empty());
        assert_eq!(settings.stream_volume_percent, 100);
        assert!(settings.catalog.recently_played.is_empty());
        assert!(!settings.swap_rear_touch_trigger_stick);
        assert!(settings.show_stream_debug_info);
        assert!(!settings.game_profile("game-id").unwrap().rear_touch_enabled);
    }
}
