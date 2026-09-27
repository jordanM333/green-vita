use crate::Stream;
use crate::api::streaming::PlaybackBackendEvent;
use crate::api::streaming::rtc::worker::{RtcWorker, RtcWorkerEvent};
use crate::api_xbox::streaming::rtc::worker;
use crate::jobs::{PollJob, poll_job};
use crate::streaming::audio_timing::TimedAudio;
use crate::streaming::input::{GamepadFrame, PointerEvent};
use crate::streaming::video::metrics::METRICS;
use crate::streaming::video::{DecodedFrame, DirectVideoOutput};
use anyhow::Result;
use bytes::Bytes;
use rtc::peer_connection::transport::RTCIceCandidateInit;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const STREAM_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
#[path = "refresh_attempt.rs"]
mod refresh_attempt;

pub(crate) struct XboxStreamingBackend {
    stream: Stream,
    worker: Option<RtcWorker>,
    microphone: crate::streaming::microphone::Microphone,
    output: Arc<DirectVideoOutput>,
    refresh_requested: bool,
    refresh_attempt: refresh_attempt::RefreshAttempt,
    retiring_worker: Option<JoinHandle<Result<()>>>,
    refresh_events: std::collections::VecDeque<PlaybackBackendEvent>,
    ice_next_poll_at: Instant,
    remote_ice_candidates: HashSet<String>,
    pending_local_ice_candidates: Vec<RTCIceCandidateInit>,
    ice_post_job: Option<JoinHandle<Result<()>>>,
    ice_poll_job: Option<JoinHandle<Result<Option<Vec<RTCIceCandidateInit>>>>>,
    keepalive_next_at: Instant,
    keepalive_job: Option<JoinHandle<Result<serde_json::Value>>>,
}

impl XboxStreamingBackend {
    pub(crate) fn start(
        stream: Stream,
        microphone: crate::streaming::microphone::Microphone,
    ) -> Result<Self> {
        let worker = worker::spawn(stream.clone(), microphone.clone())?;
        let output = Arc::clone(&worker.direct_video_output);
        Ok(Self {
            stream,
            worker: Some(worker),
            microphone,
            output,
            refresh_requested: false,
            refresh_attempt: Default::default(),
            retiring_worker: None,
            refresh_events: Default::default(),
            ice_next_poll_at: Instant::now(),
            remote_ice_candidates: HashSet::new(),
            pending_local_ice_candidates: Vec::new(),
            ice_post_job: None,
            ice_poll_job: None,
            keepalive_next_at: Instant::now(),
            keepalive_job: None,
        })
    }

    pub(crate) fn try_recv_event(&mut self) -> Option<PlaybackBackendEvent> {
        if !self.refresh_requested
            && self.retiring_worker.is_none()
            && self.worker.is_some()
            && self.output.has_produced_frame()
        {
            self.refresh_attempt.finish();
        }
        if let Some(event) = self.refresh_events.pop_front() {
            return Some(event);
        }
        loop {
            match self.worker.as_ref()?.events_rx.try_recv().ok()? {
                RtcWorkerEvent::LocalCandidates(candidates) => {
                    self.pending_local_ice_candidates.extend(candidates);
                }
                RtcWorkerEvent::Status { status } => {
                    return Some(PlaybackBackendEvent::Status(status));
                }
                RtcWorkerEvent::VideoResolution(width, height) => {
                    return Some(PlaybackBackendEvent::VideoResolution(width, height));
                }
                RtcWorkerEvent::VideoTiming(timing) => {
                    return Some(PlaybackBackendEvent::VideoTiming(timing));
                }
                RtcWorkerEvent::Closed => {
                    if self.refresh_attempt.awaiting_picture() {
                        self.refresh_attempt.failed();
                        return Some(PlaybackBackendEvent::MediaRefreshFailed(
                            "Media connection closed before its first picture. Refresh stream to retry.".into(),
                        ));
                    }
                    return Some(PlaybackBackendEvent::Closed);
                }
                RtcWorkerEvent::Error(message) => {
                    if self.refresh_attempt.awaiting_picture() {
                        self.refresh_attempt.failed();
                        return Some(PlaybackBackendEvent::MediaRefreshFailed(message));
                    }
                    return Some(PlaybackBackendEvent::Error(message));
                }
            }
        }
    }

    pub(crate) fn try_recv_audio_packets(&self) -> Option<Vec<TimedAudio<Bytes>>> {
        let batch = self.worker.as_ref()?.audio_rx.try_recv().ok()?;
        let age_us = batch.queued_at.elapsed().as_micros() as u64;
        METRICS
            .audio_batch_age_sum_us
            .fetch_add(age_us, Ordering::Relaxed);
        METRICS
            .audio_batch_age_count
            .fetch_add(1, Ordering::Relaxed);
        METRICS
            .audio_batch_age_max_us
            .fetch_max(age_us, Ordering::Relaxed);
        Some(batch.packets)
    }

    pub(crate) fn take_latest_frame(&self) -> Option<(u64, DecodedFrame)> {
        self.worker.as_ref()?.latest_frame.lock().ok()?.take()
    }

    pub(crate) fn direct_video_output(&self) -> Arc<DirectVideoOutput> {
        Arc::clone(&self.output)
    }

    pub(crate) fn send_gamepad_frame(&self, frame: GamepadFrame) {
        if let Some(worker) = &self.worker {
            worker.send_gamepad_frame(frame);
        }
    }

    pub(crate) fn send_gamepad_pulse(&self, frame: GamepadFrame) {
        if let Some(worker) = &self.worker {
            worker.send_gamepad_pulse(frame);
        }
    }

    pub(crate) fn send_pointer_event(&self, event: PointerEvent) {
        if let Some(worker) = &self.worker {
            worker.send_pointer_event(event);
        }
    }

    pub(crate) fn refresh_video(&mut self) {
        if self.stream.kind() == crate::api_xbox::session_kind::StreamKind::Home {
            // The previous action only requested an IDR on the delayed transport.
            // A manual media reconnect reuses this exact REST session. Neither
            // Stream::stop nor a new /play request is part of refresh.
            if self.retiring_worker.is_none() && self.refresh_attempt.begin(Instant::now()) {
                self.refresh_requested = true;
            }
        } else if let Some(worker) = &self.worker {
            worker.refresh_video();
        }
    }

    pub(crate) async fn maintain(&mut self) -> Option<String> {
        self.maintain_refresh().await;
        self.post_local_ice().await;
        self.poll_remote_ice().await;
        self.keep_alive().await
    }

    async fn maintain_refresh(&mut self) {
        if std::mem::take(&mut self.refresh_requested) {
            // Join old ICE tasks before posting the replacement offer: an old
            // candidate response must never be delivered to the new peer.
            if let Some(job) = self.ice_post_job.take() {
                crate::jobs::cancel(job).await;
            }
            if let Some(job) = self.ice_poll_job.take() {
                crate::jobs::cancel(job).await;
            }
            self.pending_local_ice_candidates.clear();
            self.remote_ice_candidates.clear();
            self.refresh_events.clear();
            self.refresh_events
                .push_back(PlaybackBackendEvent::MediaReset);
            // A new output identity immediately detaches old textures and
            // resets the Opus/PCM/device queues in the shell.
            self.output = Arc::new(DirectVideoOutput::new(
                crate::streaming::video::HW_OUTPUT_WIDTH,
                crate::streaming::video::HW_OUTPUT_HEIGHT,
            ));
            if let Some(worker) = self.worker.take() {
                self.retiring_worker = Some(tokio::spawn(worker.shutdown()));
            } else {
                self.start_refreshed_worker();
            }
        }
        if let Some(job) = self.retiring_worker.take() {
            match poll_job(job).await {
                PollJob::Pending(job) => self.retiring_worker = Some(job),
                PollJob::Done(Ok(())) => self.start_refreshed_worker(),
                PollJob::Done(Err(error)) => {
                    self.refresh_attempt.failed();
                    self.refresh_events
                        .push_back(PlaybackBackendEvent::MediaRefreshFailed(format!(
                            "Media refresh failed: {error:#}. Refresh stream to retry."
                        )));
                }
            }
        }
        if self.retiring_worker.is_none() && self.refresh_attempt.timed_out(Instant::now()) {
            self.refresh_events.push_back(PlaybackBackendEvent::MediaRefreshFailed(
                "Media reconnect did not produce a picture within 15 seconds. Refresh stream to retry.".into(),
            ));
        }
    }

    fn start_refreshed_worker(&mut self) {
        match worker::spawn(self.stream.clone(), self.microphone.clone()) {
            Ok(worker) => {
                self.output = Arc::clone(&worker.direct_video_output);
                self.worker = Some(worker);
                self.ice_next_poll_at = Instant::now();
            }
            Err(error) => {
                self.refresh_attempt.failed();
                self.refresh_events
                    .push_back(PlaybackBackendEvent::MediaRefreshFailed(format!(
                        "Media refresh failed: {error:#}. Refresh stream to retry."
                    )));
            }
        }
    }

    pub(crate) fn description(&self) -> String {
        "Xbox streaming session".to_owned()
    }

    pub(crate) async fn stop(self) -> Result<()> {
        let Self {
            stream,
            worker,
            ice_post_job,
            ice_poll_job,
            keepalive_job,
            retiring_worker,
            ..
        } = self;
        if let Some(job) = ice_post_job {
            crate::jobs::cancel(job).await;
        }
        if let Some(job) = ice_poll_job {
            crate::jobs::cancel(job).await;
        }
        if let Some(job) = keepalive_job {
            crate::jobs::cancel(job).await;
        }
        // A pending join is not cancellable teardown: wait until native decoder
        // ownership is released, including stop while refresh is in progress.
        if let Some(job) = retiring_worker {
            job.await??;
        }
        let stopped = match worker {
            Some(worker) => worker.shutdown().await,
            None => Ok(()),
        };
        stream.stop().await?;
        stopped?;
        eprintln!("Streaming resources released");
        Ok(())
    }

    async fn post_local_ice(&mut self) {
        if self.worker.is_none() {
            return;
        }
        if let Some(job) = self.ice_post_job.take() {
            match poll_job(job).await {
                PollJob::Pending(job) => {
                    self.ice_post_job = Some(job);
                    return;
                }
                PollJob::Done(Ok(())) => {}
                PollJob::Done(Err(error)) => {
                    eprintln!("Failed to post local ICE candidates: {error:#}");
                }
            }
        }

        if self.pending_local_ice_candidates.is_empty() {
            return;
        }

        let stream = self.stream.clone();
        let candidates = std::mem::take(&mut self.pending_local_ice_candidates);
        let count = candidates.len();
        self.ice_post_job = Some(tokio::spawn(async move {
            stream.post_ice_candidates(candidates).await?;
            eprintln!("Posted {count} local ICE candidate(s) to xCloud");
            Ok(())
        }));
    }

    async fn poll_remote_ice(&mut self) {
        if self.worker.is_none() {
            return;
        }
        if let Some(job) = self.ice_poll_job.take() {
            match poll_job(job).await {
                PollJob::Pending(job) => self.ice_poll_job = Some(job),
                PollJob::Done(result) => {
                    let response = result.unwrap_or_else(|error| {
                        eprintln!("Failed to poll remote ICE candidates: {error:#}");
                        None
                    });
                    for candidate in response.into_iter().flatten() {
                        let key = format!(
                            "{}|{}|{}",
                            candidate.candidate,
                            candidate.sdp_mid.as_deref().unwrap_or(""),
                            candidate.sdp_mline_index.unwrap_or(0)
                        );
                        if self.remote_ice_candidates.insert(key)
                            && let Some(worker) = &self.worker
                        {
                            worker.add_remote_candidate(candidate);
                        }
                    }
                }
            }
        }

        if Instant::now() < self.ice_next_poll_at || self.ice_poll_job.is_some() {
            return;
        }

        let stream = self.stream.clone();
        self.ice_next_poll_at = Instant::now() + Duration::from_secs(1);
        self.ice_poll_job = Some(tokio::spawn(
            async move { stream.poll_ice_candidates().await },
        ));
    }

    async fn keep_alive(&mut self) -> Option<String> {
        if let Some(job) = self.keepalive_job.take() {
            match poll_job(job).await {
                PollJob::Pending(job) => {
                    self.keepalive_job = Some(job);
                    return None;
                }
                PollJob::Done(Ok(response)) => {
                    if let Some(code) = response.get("code").and_then(serde_json::Value::as_str)
                        && matches!(code, "SessionNotActive" | "SessionNotFound")
                    {
                        return Some(code.to_owned());
                    }
                }
                PollJob::Done(Err(error)) => {
                    eprintln!("Failed to send xCloud keepalive: {error:#}");
                }
            }
        }

        if Instant::now() < self.keepalive_next_at {
            return None;
        }
        self.keepalive_next_at = Instant::now() + STREAM_KEEPALIVE_INTERVAL;

        let stream = self.stream.clone();
        self.keepalive_job = Some(tokio::spawn(async move { stream.send_keepalive().await }));
        None
    }
}
