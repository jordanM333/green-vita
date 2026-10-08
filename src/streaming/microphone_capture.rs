//! Blocking Vita capture and Opus encoding run outside the video/RTC thread.
use super::microphone::Microphone;
#[cfg(target_os = "vita")]
use super::microphone::VoiceClip;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
        let thread = std::thread::Builder::new()
            .name("green-vita-mic".into())
            .spawn(move || {
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
        Ok(Self {
            stop,
            thread: Some(thread),
            microphone,
        })
    }
}
impl Drop for MicrophoneCapture {
    fn drop(&mut self) {
        self.microphone.set_ready(false);
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(target_os = "vita"))]
fn capture(
    _: &Microphone,
    _: super::microphone::CaptureTicket,
    _: &AtomicBool,
    _: Instant,
) -> anyhow::Result<()> {
    anyhow::bail!("Vita audio input required")
}

#[cfg(target_os = "vita")]
fn capture(
    mic: &Microphone,
    ticket: super::microphone::CaptureTicket,
    stop: &AtomicBool,
    clock: Instant,
) -> anyhow::Result<()> {
    use super::microphone::MicInput;
    use anyhow::{Context, ensure};
    use std::collections::VecDeque;
    // Voice port: same hardware format as SDL's Vita capture driver, 512 mono
    // s16 samples at 16 kHz, reframed from 32 ms reads into 20 ms Opus packets.
    // The raw port's accepted formats are not documented, so try 16 kHz first
    // and then 48 kHz (decimated 3:1 to the same 16 kHz encoder input).
    const VOICE: (u32, i32, i32) = (vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_VOICE, 512, 16000);
    const RAW: [(u32, i32, i32); 4] = [
        (vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_RAW, 512, 16000),
        (vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_RAW, 256, 16000),
        (vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_RAW, 256, 48000),
        (vitasdk_sys::SCE_AUDIO_IN_PORT_TYPE_RAW, 512, 48000),
    ];
    let open = |(kind, grain, freq): (u32, i32, i32)| {
        // SAFETY: fixed mono S16 format from the list above; no pointers passed.
        unsafe {
            vitasdk_sys::sceAudioInOpenPort(
                kind,
                grain,
                freq,
                vitasdk_sys::SCE_AUDIO_IN_PARAM_FORMAT_S16_MONO,
            )
        }
    };
    let mut refused = Vec::new();
    let mut opened = None;
    if mic.input() == MicInput::Raw {
        for format in RAW {
            let port = open(format);
            if port >= 0 {
                opened = Some((port, format, "raw port"));
                break;
            }
            refused.push(format!("{}@{}k {port:#x}", format.1, format.2 / 1000));
        }
    }
    let (port, (kind, grain, freq), name) = match opened {
        Some(opened) => opened,
        None => {
            let port = open(VOICE);
            ensure!(port >= 0, "capture open failed ({port:#x})");
            (port, VOICE, "voice port")
        }
    };
    // SAFETY: status queries take no pointers and do not touch the port.
    let (adopt, mute) = unsafe {
        (
            vitasdk_sys::sceAudioInGetAdopt(kind),
            vitasdk_sys::sceAudioInGetStatus(vitasdk_sys::SCE_AUDIO_IN_GETSTATUS_MUTE as i32),
        )
    };
    let mut detail = format!("{name} {grain}@{}k adopt:{adopt} mute:{mute}", freq / 1000);
    if !refused.is_empty() {
        detail = format!("raw port refused ({}), {detail}", refused.join(", "));
    }
    eprintln!("Microphone capture opened: {detail}");
    mic.set_opened(ticket, detail);
    let decimate = (freq / 16000) as usize;
    struct Port(i32);
    impl Drop for Port {
        fn drop(&mut self) {
            // SAFETY: sole successfully opened port owner; all input calls have returned.
            unsafe {
                vitasdk_sys::sceAudioInReleasePort(self.0);
            }
        }
    }
    let _port = Port(port);
    let mut encoder = super::voice_encoder::VoiceEncoder::new().context("Opus encoder")?;
    let mut pending = VecDeque::with_capacity(832);
    let mut input = vec![0i16; grain as usize];
    let mut decimated = Vec::with_capacity(input.len());
    let (mut sum, mut summed) = (0i32, 0usize);
    let mut samples = [0i16; 320];
    let mut timestamp = (clock.elapsed().as_micros() * 48 / 1000) as u32;
    while !stop.load(Ordering::Relaxed) && mic.begin_capture() == Some(ticket) {
        let input_started = Instant::now();
        // SAFETY: port is live, and input is an initialized writable buffer of
        // exactly `grain` i16 samples matching the open format; synchronous
        // input must not retain its pointer.
        let result = unsafe { vitasdk_sys::sceAudioInInput(port, input.as_mut_ptr().cast()) };
        ensure!(result >= 0, "capture read failed ({result:#x})");
        if mic.begin_capture() != Some(ticket) {
            break;
        }
        if input_started.elapsed() > Duration::from_millis(80) {
            pending.clear();
            (sum, summed) = (0, 0);
            timestamp = (clock.elapsed().as_micros() * 48 / 1000) as u32;
            continue;
        }
        if decimate > 1 {
            // Average each run of `decimate` samples: a plain low-pass that is
            // adequate for speech into the 16 kHz voice encoder.
            decimated.clear();
            for sample in &input {
                sum += i32::from(*sample);
                summed += 1;
                if summed == decimate {
                    decimated.push((sum / decimate as i32) as i16);
                    (sum, summed) = (0, 0);
                }
            }
            pending.extend(decimated.iter().copied());
        } else {
            pending.extend(input.iter().copied());
        }
        while pending.len() >= samples.len() {
            for sample in &mut samples {
                *sample = pending.pop_front().unwrap();
            }
            let peak = samples
                .iter()
                .map(|s| (*s as f32).abs() / 32768.0)
                .fold(0.0f32, f32::max);
            let opus = encoder.encode(&samples)?;
            mic.publish(
                VoiceClip {
                    ticket,
                    captured_at: input_started,
                    timestamp,
                    opus,
                },
                peak,
            );
            // Opus RTP always uses the 48 kHz clock, even for 16 kHz capture.
            timestamp = timestamp.wrapping_add(960);
        }
    }
    Ok(())
}
