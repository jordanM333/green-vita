use crate::api::streaming::{PlaybackBackend, PlaybackBackendEvent};
use crate::settings::Settings;
use crate::streaming::input::{GamepadFrame, PointerEvent};
use crate::streaming::video::{DecodedFrame, DirectVideoOutput};
use crate::{Stream, StreamKind};
use anyhow::Result;
use bytes::Bytes;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy)]
pub(super) enum StreamReturnTarget {
    Titles(usize),
    Consoles(usize),
}

pub(crate) struct StreamingSession {
    pub(crate) paused: bool,
    pub(crate) microphone: crate::streaming::microphone::Microphone,
    pub(crate) status: String,
    pub(crate) hint_started_at: Instant,
    pub(crate) video_startup: crate::streaming::video::startup::VideoStartup,
    pub(in crate::app) pause_selected: usize,
    pub(in crate::app) title_id: Option<String>,
    pub(super) return_target: StreamReturnTarget,
    backend: PlaybackBackend,
    pub(super) restart_target: super::StreamStartTarget,
    pub(super) video_timing: Option<crate::streaming::video::freshness::VideoTiming>,
    latest_video_frame: Option<u64>,
    current_video_frame: Option<DecodedFrame>,
    stream_video_size: Option<(u32, u32)>,
    pending_audio_packets: Vec<Bytes>,
    ignore_confirm_until_release: bool,
}

impl StreamingSession {
    pub(super) fn start_xbox(
        stream: Stream,
        kind: StreamKind,
        title_id: Option<String>,
        return_selected: usize,
        restart_target: super::StreamStartTarget,
    ) -> Result<Self> {
        let microphone = crate::streaming::microphone::Microphone::default();
        let backend = PlaybackBackend::start_xbox(stream, microphone.clone())?;
        let return_target = match kind {
            StreamKind::Cloud => StreamReturnTarget::Titles(return_selected),
            StreamKind::Home => StreamReturnTarget::Consoles(return_selected),
        };
        Ok(Self {
            paused: false,
            microphone,
            status: "Starting streaming backend".to_owned(),
            hint_started_at: Instant::now(),
            video_startup: crate::streaming::video::startup::VideoStartup::new(Instant::now()),
            pause_selected: 0,
            title_id,
            return_target,
            backend,
            restart_target,
            video_timing: None,
            latest_video_frame: None,
            current_video_frame: None,
            stream_video_size: None,
            pending_audio_packets: Vec::new(),
            ignore_confirm_until_release: true,
        })
    }

    pub(crate) fn can_refresh(&self) -> bool { matches!(self.restart_target.kind, StreamKind::Home) }

    pub(crate) fn measured_delay_ms(&self) -> Option<u64> {
        self.video_timing.filter(|timing|
            timing.received_at.elapsed() <= std::time::Duration::from_millis(1500))
            .map(|timing| timing.added_delay_ms)
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.hint_started_at = Instant::now();
    }

    pub(crate) fn take_audio_packets(&mut self) -> Vec<Bytes> {
        std::mem::take(&mut self.pending_audio_packets)
    }

    pub(crate) fn video_frame(&self) -> Option<(u64, &DecodedFrame)> {
        Some((self.latest_video_frame?, self.current_video_frame.as_ref()?))
    }

    pub(crate) fn video_size(&self) -> Option<(u32, u32)> {
        self.stream_video_size
    }

    pub(crate) fn direct_video_output(&self) -> Arc<DirectVideoOutput> {
        self.backend.direct_video_output()
    }

    pub(crate) fn send_gamepad_frame(&mut self, mut frame: GamepadFrame, settings: &Settings) {
        if self.ignore_confirm_until_release {
            if frame.a > 0.0 {
                return;
            }
            self.ignore_confirm_until_release = false;
        }

        let swap_shoulders_and_triggers = self
            .title_id
            .as_deref()
            .and_then(|title_id| settings.game_profile(title_id))
            .is_some_and(|profile| profile.swap_shoulders_and_triggers);
        if swap_shoulders_and_triggers {
            std::mem::swap(&mut frame.left_shoulder, &mut frame.left_trigger);
            std::mem::swap(&mut frame.right_shoulder, &mut frame.right_trigger);
        }
        self.backend.send_gamepad_frame(frame);
    }

    pub(crate) fn front_touch_auxiliary_buttons(&self, settings: &Settings) -> bool {
        self.title_id
            .as_deref()
            .and_then(|title_id| settings.game_profile(title_id))
            .is_some_and(|profile| profile.front_touch_auxiliary_buttons)
    }

    pub(crate) fn rear_touch_enabled(&self, settings: &Settings) -> bool {
        self.title_id
            .as_deref()
            .and_then(|title_id| settings.game_profile(title_id))
            .is_none_or(|profile| profile.rear_touch_enabled)
    }

    pub(crate) fn press_guide_button(&mut self) {
        self.ignore_confirm_until_release = true;
        self.backend.send_gamepad_pulse(GamepadFrame {
            nexus: 1.0,
            ..Default::default()
        });
    }

    pub(crate) fn send_pointer_event(&self, event: PointerEvent) {
        self.backend.send_pointer_event(event);
    }

    pub(super) fn drain_backend_events(&mut self) -> (bool, Option<String>) {
        let mut events = Vec::new();
        while let Some(event) = self.backend.try_recv_event() {
            events.push(event);
        }
        while let Some(mut packets) = self.backend.try_recv_audio_packets() {
            self.pending_audio_packets.append(&mut packets);
        }

        if let Some((frame_id, frame)) = self.backend.take_latest_frame() {
            self.latest_video_frame = Some(frame_id);
            self.current_video_frame = Some(frame);
        }

        if !self.video_startup.picture_seen() {
            self.video_startup.observe_picture(self.latest_video_frame.is_some()
                || self.backend.direct_video_output().has_produced_frame());
        }

        let mut closed = false;
        let mut error = None;
        for event in events {
            match event {
                PlaybackBackendEvent::Status(status) => self.status = status,
                PlaybackBackendEvent::VideoResolution(width, height) => {
                    self.stream_video_size = Some((width, height));
                }
                PlaybackBackendEvent::VideoTiming(timing) => self.video_timing = Some(timing),
                PlaybackBackendEvent::Closed => closed = true,
                PlaybackBackendEvent::Error(message) => error = Some(message),
            }
        }

        (closed, error)
    }

    pub(super) async fn maintain_backend(&mut self) -> Option<String> {
        self.backend.maintain().await
    }

    pub(super) fn backend_description(&self) -> String {
        self.backend.description()
    }

    pub(super) async fn stop(self) -> Result<()> {
        self.microphone.set_ready(false);
        self.backend.stop().await
    }
}
