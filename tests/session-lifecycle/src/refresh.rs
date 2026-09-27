//! Actual XboxStreamingBackend; only hardware-worker and HTTP boundaries are fakes.
//! This proves teardown/order/session ownership, not Xbox SDP acceptance.
use crate::{Stream, session_kind::StreamKind};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
#[path = "../../../src/api_xbox/streaming/backend.rs"]
pub mod backend;

pub mod streaming {
    pub mod audio_timing {
        pub struct TimedAudio<T>(pub T);
    }
    pub mod input {
        pub struct GamepadFrame;
        pub struct PointerEvent;
    }
    pub mod microphone {
        #[derive(Clone, Default)]
        pub struct Microphone(pub std::sync::Arc<super::super::Control>);
    }
    pub mod video {
        use std::sync::atomic::{AtomicBool, Ordering};
        pub const HW_OUTPUT_WIDTH: u32 = 960;
        pub const HW_OUTPUT_HEIGHT: u32 = 544;
        pub struct DecodedFrame;
        pub struct DirectVideoOutput {
            pub produced: AtomicBool,
        }
        impl DirectVideoOutput {
            pub fn new(_: u32, _: u32) -> Self {
                Self {
                    produced: AtomicBool::new(false),
                }
            }
            pub fn has_produced_frame(&self) -> bool {
                self.produced.load(Ordering::SeqCst)
            }
        }
        pub mod metrics {
            use std::sync::atomic::AtomicU64;
            pub struct Metrics {
                pub audio_batch_age_sum_us: AtomicU64,
                pub audio_batch_age_count: AtomicU64,
                pub audio_batch_age_max_us: AtomicU64,
            }
            pub static METRICS: Metrics = Metrics {
                audio_batch_age_sum_us: AtomicU64::new(0),
                audio_batch_age_count: AtomicU64::new(0),
                audio_batch_age_max_us: AtomicU64::new(0),
            };
        }
    }
}
pub mod api {
    pub mod streaming {
        pub enum PlaybackBackendEvent {
            Status(String),
            VideoResolution(u32, u32),
            VideoTiming(()),
            MediaReset,
            MediaRefreshFailed(String),
            Closed,
            Error(String),
        }
        pub mod rtc {
            pub mod worker {
                pub use crate::refresh::worker::*;
            }
        }
    }
}
#[derive(Default)]
pub struct Control {
    active: AtomicUsize,
    starts: AtomicUsize,
    block_shutdown: AtomicBool,
    fail_start: AtomicBool,
    gate: tokio::sync::Notify,
    session_paths: Mutex<Vec<String>>,
    local_refreshes: AtomicUsize,
}
pub mod worker {
    use super::*;
    use rtc::peer_connection::transport::RTCIceCandidateInit;
    use std::{
        sync::mpsc::{Receiver, channel},
        time::Instant,
    };
    use streaming::{
        audio_timing::TimedAudio,
        input::{GamepadFrame, PointerEvent},
        microphone::Microphone,
        video::{DecodedFrame, DirectVideoOutput},
    };
    pub enum RtcWorkerEvent {
        LocalCandidates(Vec<RTCIceCandidateInit>),
        Status { status: String },
        VideoResolution(u32, u32),
        VideoTiming(()),
        Closed,
        Error(String),
    }
    pub struct TimedAudioBatch {
        pub queued_at: Instant,
        pub packets: Vec<TimedAudio<bytes::Bytes>>,
    }
    pub struct RtcWorker {
        pub events_rx: Receiver<RtcWorkerEvent>,
        pub audio_rx: Receiver<TimedAudioBatch>,
        pub latest_frame: Arc<Mutex<Option<(u64, DecodedFrame)>>>,
        pub direct_video_output: Arc<DirectVideoOutput>,
        control: Arc<Control>,
    }
    pub fn spawn(stream: Stream, microphone: Microphone) -> anyhow::Result<RtcWorker> {
        let c = microphone.0;
        assert_eq!(
            c.active.load(Ordering::SeqCst),
            0,
            "native decoder must be released before replacement"
        );
        if c.fail_start.load(Ordering::SeqCst) {
            anyhow::bail!("injected allocation failure");
        }
        c.active.fetch_add(1, Ordering::SeqCst);
        c.starts.fetch_add(1, Ordering::SeqCst);
        c.session_paths.lock().unwrap().push(stream.session_path());
        Ok(RtcWorker {
            events_rx: channel().1,
            audio_rx: channel().1,
            latest_frame: Default::default(),
            direct_video_output: Arc::new(DirectVideoOutput::new(960, 544)),
            control: c,
        })
    }
    impl RtcWorker {
        pub async fn shutdown(self) -> anyhow::Result<()> {
            if self.control.block_shutdown.load(Ordering::SeqCst) {
                self.control.gate.notified().await;
            }
            Ok(())
        }
        pub fn refresh_video(&self) {
            self.control.local_refreshes.fetch_add(1, Ordering::SeqCst);
        }
        pub fn add_remote_candidate(&self, _: RTCIceCandidateInit) {}
        pub fn send_gamepad_frame(&self, _: GamepadFrame) {}
        pub fn send_gamepad_pulse(&self, _: GamepadFrame) {}
        pub fn send_pointer_event(&self, _: PointerEvent) {}
    }
    impl Drop for RtcWorker {
        fn drop(&mut self) {
            self.control.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
fn start(
    kind: StreamKind,
) -> (
    backend::XboxStreamingBackend,
    Arc<Control>,
    crate::api_xbox::api::ApiClient,
) {
    let api = crate::api_xbox::api::ApiClient::default();
    let c = Arc::new(Control::default());
    let s = Stream::new(
        api.clone(),
        crate::api_xbox::auth::EndpointCredentials,
        crate::stream::StartStreamResponse {
            session_path: format!("/v5/sessions/{}/owned", kind.as_path()),
        },
        kind,
    );
    let b = backend::XboxStreamingBackend::start(s, streaming::microphone::Microphone(c.clone()))
        .unwrap();
    (b, c, api)
}
async fn pump(b: &mut backend::XboxStreamingBackend) {
    b.maintain().await;
    tokio::task::yield_now().await;
}
#[tokio::test]
async fn home_refresh_retires_old_media_before_replacing_and_reuses_the_owned_session() {
    let (mut b, c, api) = start(StreamKind::Home);
    for n in 1..=100 {
        let old = b.direct_video_output();
        old.produced.store(true, Ordering::SeqCst);
        while b.try_recv_event().is_some() {}
        c.block_shutdown.store(true, Ordering::SeqCst);
        for _ in 0..20 {
            b.refresh_video();
        }
        pump(&mut b).await;
        assert!(
            !Arc::ptr_eq(&old, &b.direct_video_output()),
            "audio/video generation must change at refresh start"
        );
        assert!(matches!(
            b.try_recv_event(),
            Some(api::streaming::PlaybackBackendEvent::MediaReset)
        ));
        for _ in 0..20 {
            b.refresh_video();
            pump(&mut b).await;
        }
        assert_eq!(c.starts.load(Ordering::SeqCst), n);
        assert_eq!(c.active.load(Ordering::SeqCst), 1);
        c.block_shutdown.store(false, Ordering::SeqCst);
        c.gate.notify_one();
        for _ in 0..4 {
            pump(&mut b).await;
        }
        assert_eq!(c.starts.load(Ordering::SeqCst), n + 1);
        // While acquiring a first picture, repeated clicks still coalesce.
        for _ in 0..20 {
            b.refresh_video();
            pump(&mut b).await;
        }
        assert_eq!(c.starts.load(Ordering::SeqCst), n + 1);
    }
    b.stop().await.unwrap();
    assert_eq!(c.active.load(Ordering::SeqCst), 0);
    assert!(
        c.session_paths
            .lock()
            .unwrap()
            .iter()
            .all(|p| p == "/v5/sessions/home/owned")
    );
    assert!(
        api.0
            .lock()
            .unwrap()
            .iter()
            .all(|(m, p)| *m != reqwest::Method::DELETE
                && !p.ends_with("/play")
                && !p.ends_with("/connect"))
    );
}
#[tokio::test]
async fn stop_during_refresh_joins_teardown_without_starting_replacement() {
    let (mut b, c, _) = start(StreamKind::Home);
    c.block_shutdown.store(true, Ordering::SeqCst);
    b.refresh_video();
    pump(&mut b).await;
    let stop = tokio::spawn(b.stop());
    tokio::task::yield_now().await;
    assert!(!stop.is_finished());
    c.gate.notify_one();
    stop.await.unwrap().unwrap();
    assert_eq!(c.starts.load(Ordering::SeqCst), 1);
    assert_eq!(c.active.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn failed_refresh_keeps_reset_event_and_can_retry_without_new_game() {
    let (mut b, c, _) = start(StreamKind::Home);
    c.fail_start.store(true, Ordering::SeqCst);
    b.refresh_video();
    for _ in 0..4 {
        pump(&mut b).await;
    }
    assert!(matches!(
        b.try_recv_event(),
        Some(api::streaming::PlaybackBackendEvent::MediaReset)
    ));
    assert!(
        matches!(b.try_recv_event(),Some(api::streaming::PlaybackBackendEvent::MediaRefreshFailed(s)) if s.contains("failed"))
    );
    c.fail_start.store(false, Ordering::SeqCst);
    b.refresh_video();
    pump(&mut b).await;
    assert_eq!(c.starts.load(Ordering::SeqCst), 2);
    b.stop().await.unwrap();
}
#[tokio::test]
async fn cloud_refresh_retains_connection_and_cloud_stop_remains_owned() {
    let (mut b, c, api) = start(StreamKind::Cloud);
    let old = b.direct_video_output();
    b.refresh_video();
    pump(&mut b).await;
    assert!(Arc::ptr_eq(&old, &b.direct_video_output()));
    assert_eq!(c.starts.load(Ordering::SeqCst), 1);
    assert_eq!(c.local_refreshes.load(Ordering::SeqCst), 1);
    b.stop().await.unwrap();
    assert!(
        api.0
            .lock()
            .unwrap()
            .contains(&(reqwest::Method::DELETE, "/v5/sessions/cloud/owned".into()))
    );
}
