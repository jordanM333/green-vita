//! Blocking Vita capture and Opus encoding run outside the video/RTC thread.
use super::microphone::Microphone;
#[cfg(target_os = "vita")]
use super::microphone::VoiceClip;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

pub(crate) struct MicrophoneCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    microphone: Microphone,
}
impl MicrophoneCapture {
    pub(crate) fn spawn(microphone: Microphone) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let mic = microphone.clone();
        let thread = std::thread::Builder::new().name("green-vita-mic".into()).spawn(move || {
            let clock = Instant::now();
            while !worker_stop.load(Ordering::Relaxed) {
                if let Some(ticket) = mic.begin_capture() {
                    if let Err(error) = capture(&mic, ticket, &worker_stop, clock) {
                        mic.fail(error.to_string());
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        })?;
        Ok(Self { stop, thread: Some(thread), microphone })
    }
}
impl Drop for MicrophoneCapture {
    fn drop(&mut self) {
        self.microphone.set_ready(false);
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(not(target_os = "vita"))]
fn capture(_: &Microphone, _: super::microphone::CaptureTicket, _: &AtomicBool, _: Instant) -> anyhow::Result<()> {
    anyhow::bail!("Vita audio input required")
}

#[cfg(target_os = "vita")]
fn capture(mic: &Microphone, ticket: super::microphone::CaptureTicket, stop: &AtomicBool, clock: Instant) -> anyhow::Result<()> {
    use anyhow::{ensure, Context};
    use std::collections::VecDeque;
    // Same hardware format as SDL's Vita capture driver: 512 mono s16 samples,
    // 16 kHz. Reframe its 32 ms reads into 20 ms Opus packets without resampling.
    let port = unsafe { vitasdk_sys::sceAudioInOpenPort(
        vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_VOICE, 512, 16000,
        vitasdk_sys::SCE_AUDIO_IN_PARAM_FORMAT_S16_MONO) };
    ensure!(port >= 0, "capture open failed ({port:#x})");
    struct Port(i32);
    impl Drop for Port { fn drop(&mut self) { unsafe { vitasdk_sys::sceAudioInReleasePort(self.0); } } }
    let _port = Port(port);
    let mut encoder = super::voice_encoder::VoiceEncoder::new().context("Opus encoder")?;
    let mut pending = VecDeque::with_capacity(832);
    let mut input = [0i16; 512];
    let mut samples = [0i16; 320];
    let mut timestamp = (clock.elapsed().as_micros() * 48 / 1000) as u32;
    while !stop.load(Ordering::Relaxed) && mic.begin_capture() == Some(ticket) {
        let input_started = Instant::now();
        let result = unsafe { vitasdk_sys::sceAudioInInput(port, input.as_mut_ptr().cast()) };
        ensure!(result >= 0, "capture read failed ({result:#x})");
        if mic.begin_capture() != Some(ticket) { break; }
        if input_started.elapsed() > Duration::from_millis(80) {
            pending.clear();
            timestamp = (clock.elapsed().as_micros() * 48 / 1000) as u32;
            continue;
        }
        pending.extend(input);
        while pending.len() >= samples.len() {
            for sample in &mut samples { *sample = pending.pop_front().unwrap(); }
            let peak = samples.iter().map(|s| (*s as f32).abs() / 32768.0).fold(0.0f32, f32::max);
            let opus = encoder.encode(&samples)?;
            mic.publish(VoiceClip { ticket, captured_at: input_started, timestamp, opus }, peak);
            // Opus RTP always uses the 48 kHz clock, even for 16 kHz capture.
            timestamp = timestamp.wrapping_add(960);
        }
    }
    Ok(())
}
