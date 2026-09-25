#![allow(dead_code)]
#[path = "../../../src/catalog_preferences.rs"]
mod catalog_preferences;
#[path = "../../../src/streaming/microphone.rs"]
mod microphone;
#[path = "../../../src/streaming/mic_button.rs"]
mod mic_button;
#[path = "../../../src/streaming/audio_gain.rs"]
mod audio_gain;
#[path = "../../../src/streaming/audio_timing.rs"]
mod audio_timing;
#[path = "../../../src/streaming/video/startup.rs"]
mod video_startup;
#[path = "../../../src/settings.rs"]
mod settings;
mod fs_utils {
    pub fn write_file_truncating(_: &str, _: String) -> anyhow::Result<()> { panic!("host tests must not write Vita settings") }
}

#[path = "../../../src/api_xbox/collection_order.rs"]
mod collection_order;
#[path = "../../../src/api_xbox/chat_sdp.rs"]
mod chat_sdp;

#[path = "../../../vendor/rtc/src/peer_connection/receive_bandwidth.rs"]
mod receive_bandwidth;
