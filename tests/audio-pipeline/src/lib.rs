//! Production renderer/worker/libopus; SDL device output is a recording sink.
//! This does not emulate Vita DAC timing, audio hardware, or audible latency.
#![allow(dead_code)]
extern crate self as sdl2;
#[path = "../../../src/streaming/audio_gain.rs"]
mod audio_gain;
#[path = "../../../src/streaming/audio_timing.rs"]
mod audio_timing;
#[path = "../../../src/streaming/audio.rs"]
mod audio_renderer;
#[path = "../../../src/streaming/video/metrics.rs"]
pub(crate) mod metrics;
mod streaming { pub mod video {
    pub(crate) use crate::metrics;
    pub mod trace { pub fn record(_: &'static str, _: u32, _: u64) {} }
} }
pub struct AudioSubsystem;
impl AudioSubsystem {
    pub fn open_queue<T>(&self, _: Option<&str>, desired: &audio::AudioSpecDesired) -> Result<audio::AudioQueue<T>, String> {
        Ok(audio::AudioQueue { spec: audio::AudioSpec { freq: desired.freq.unwrap(), channels: desired.channels.unwrap() },
            samples: Default::default() })
    }
}
pub mod audio {
    use std::cell::RefCell;
    pub struct AudioSpecDesired { pub freq: Option<i32>, pub channels: Option<u8>, pub samples: Option<u16> }
    pub struct AudioSpec { pub freq: i32, pub channels: u8 }
    pub struct AudioQueue<T> { pub spec: AudioSpec, pub samples: RefCell<Vec<T>> }
    impl<T: Clone> AudioQueue<T> {
        pub fn spec(&self) -> &AudioSpec { &self.spec }
        pub fn size(&self) -> u32 { (self.samples.borrow().len() * size_of::<T>()) as u32 }
        pub fn pause(&self) {}
        pub fn resume(&self) {}
        pub fn clear(&self) { self.samples.borrow_mut().clear(); }
        pub fn queue_audio(&self, data: &[T]) -> Result<(), String> {
            self.samples.borrow_mut().extend_from_slice(data); Ok(())
        }
    }
}
