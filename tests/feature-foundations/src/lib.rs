#![allow(dead_code)]
#[path = "../../../src/catalog_preferences.rs"]
mod catalog_preferences;
#[path = "../../../src/streaming/microphone.rs"]
mod microphone;
#[path = "../../../src/settings.rs"]
mod settings;
mod fs_utils {
    pub fn write_file_truncating(_: &str, _: String) -> anyhow::Result<()> { panic!("host tests must not write Vita settings") }
}
