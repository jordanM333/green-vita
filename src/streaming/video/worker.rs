use super::decoder::{DecodedPicture, HwVideoDecoder};
use super::metrics;
use super::{DecodedFrame, DecoderConfig, DirectVideoOutput, DirectVideoTargetGuard};
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::policy::{AU_PRESSURE_AGE, AU_QUEUE_CAPACITY, QueueReservation, poll_before_input};

struct QueuedAccessUnit {
    data: Vec<u8>,
    queued_at: Instant,
    received_at: Instant,
    rtp_timestamp: u32,
    generation: u64,
    reservation: Option<QueueReservation>,
}

enum DecoderCommand {
    Stop,
}

pub(crate) type DecodeResult = Result<DecodedFrame, String>;

pub(crate) enum SubmitResult {
    Submitted,
    QueueFull,
    Disconnected,
}

/// Shared state owned by the decoder thread; grouping does not change the
/// producer's Arc ownership or hold any mutex between native calls.
struct DecodeState {
    generation: Arc<AtomicU64>,
    recovery_needed: Arc<AtomicU64>,
    latest_result: Arc<Mutex<Option<DecodeResult>>>,
    result_ready: Arc<tokio::sync::Notify>,
    direct_output: Arc<DirectVideoOutput>,
}

pub struct VideoDecodeWorker {
    thread: Option<std::thread::JoinHandle<()>>,
    direct_output: Arc<DirectVideoOutput>,
    access_units: Sender<QueuedAccessUnit>,
    // Used only by the single RTP producer to discard queued work once a
    // complete replacement IDR is in hand. The hardware thread may hold one AU.
    queued_access_units: Receiver<QueuedAccessUnit>,
    queued_bytes: Arc<AtomicUsize>,
    commands: Sender<DecoderCommand>,
    generation: Arc<AtomicU64>,
    recovery_needed: Arc<AtomicU64>,
    pub(crate) latest_result: Arc<Mutex<Option<DecodeResult>>>,
    pub(crate) result_ready: Arc<tokio::sync::Notify>,
}

impl VideoDecodeWorker {
    pub fn spawn(config: DecoderConfig, direct_output: Arc<DirectVideoOutput>) -> Result<Self> {
        let (access_units, worker_access_units) = bounded(AU_QUEUE_CAPACITY);
        let queued_access_units = worker_access_units.clone();
        let (commands, worker_commands) = bounded(1);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let recovery_needed = Arc::new(AtomicU64::new(0));
        let worker_recovery_needed = Arc::clone(&recovery_needed);
        let latest_result = Arc::new(Mutex::new(None));
        let worker_latest_result = Arc::clone(&latest_result);
        let result_ready = Arc::new(tokio::sync::Notify::new());
        let worker_result_ready = Arc::clone(&result_ready);
        let worker_direct_output = Arc::clone(&direct_output);

        let ready_output = Arc::clone(&direct_output);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("green-vita-video-decode".to_owned())
            .spawn(move || {
                #[cfg(target_os = "vita")]
                pin_decoder_thread();
                // Native handles are created, used and destroyed by their owner.
                // No undocumented cross-thread decoder-handle transfer is needed.
                let decoder = match HwVideoDecoder::new(config) {
                    Ok(decoder) => decoder,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("{error:#}")));
                        return;
                    }
                };
                ready_output.decoder_ready.store(true, Ordering::Release);
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                run_decode_loop(
                    worker_access_units,
                    worker_commands,
                    decoder,
                    config,
                    DecodeState {
                        generation: worker_generation,
                        recovery_needed: worker_recovery_needed,
                        latest_result: worker_latest_result,
                        result_ready: worker_result_ready,
                        direct_output: worker_direct_output,
                    },
                );
                ready_output.decoder_ready.store(false, Ordering::Release);
            })
            .context("failed to spawn video decode worker")?;
        if let Err(error) = ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r) {
            let _ = thread.join();
            anyhow::bail!("failed to initialize hardware H264 decoder: {error}");
        }

        Ok(Self {
            thread: Some(thread),
            direct_output,
            access_units,
            queued_access_units,
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            commands,
            generation,
            recovery_needed,
            latest_result,
            result_ready,
        })
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.commands.send(DecoderCommand::Stop);
            // The decoder owns a scarce Vita resource. Finish its destruction
            // before any replacement stream can allocate another instance.
            if thread.join().is_err() {
                eprintln!("Video decode worker panicked during shutdown");
            }
        }
        while let Ok(old) = self.queued_access_units.try_recv() {
            drop(old);
        }
        metrics::METRICS.au_queue_depth.store(0, Ordering::Relaxed);
    }

    /// `complete_at`: when the AU's last packet was received. The local video
    /// budget measures queueing from there (HA06), not transmission time.
    pub fn submit_access_unit(
        &self,
        data: Vec<u8>,
        complete_at: Instant,
        rtp_timestamp: u32,
    ) -> SubmitResult {
        // The producer retains a receiver for keyframe cutover, so channel
        // connectivity alone no longer tells us whether the worker is alive.
        if self
            .thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
        {
            return SubmitResult::Disconnected;
        }
        let Some(reservation) = QueueReservation::acquire(&self.queued_bytes, data.len()) else {
            super::trace::record("queue_byte_limit", rtp_timestamp, data.len() as u64);
            metrics::METRICS.queue_full.fetch_add(1, Ordering::Relaxed);
            return SubmitResult::QueueFull;
        };
        let access_unit = QueuedAccessUnit {
            data,
            queued_at: Instant::now(),
            received_at: complete_at,
            rtp_timestamp,
            generation: self.generation.load(Ordering::Acquire),
            reservation: Some(reservation),
        };
        match self.access_units.try_send(access_unit) {
            Ok(()) => {
                let depth = self.access_units.len() as u64;
                super::trace::record("au_queue_depth", rtp_timestamp, depth);
                metrics::METRICS
                    .au_queue_depth
                    .store(depth, Ordering::Relaxed);
                metrics::METRICS
                    .au_queue_max
                    .fetch_max(depth, Ordering::Relaxed);
                SubmitResult::Submitted
            }
            Err(TrySendError::Full(_)) => {
                super::trace::record(
                    "queue_frame_limit",
                    rtp_timestamp,
                    self.access_units.len() as u64,
                );
                metrics::METRICS.queue_full.fetch_add(1, Ordering::Relaxed);
                SubmitResult::QueueFull
            }
            Err(TrySendError::Disconnected(_)) => SubmitResult::Disconnected,
        }
    }

    pub(crate) fn queued_frames(&self) -> usize {
        self.access_units.len()
    }

    pub(crate) fn media_admits(&self, timestamp: u32, random_access: bool, now: Instant) -> bool {
        self.direct_output
            .live_edge
            .lock()
            .is_ok_and(|edge| edge.admit(timestamp, random_access, now))
    }

    pub(crate) fn media_submitted(&self, timestamp: u32, random_access: bool, now: Instant) {
        if let Ok(mut edge) = self.direct_output.live_edge.lock() {
            edge.submitted(timestamp, random_access, now);
        }
    }

    pub(crate) fn media_damage(&self) {
        if let Ok(mut edge) = self.direct_output.live_edge.lock() {
            edge.damage();
        }
    }

    pub(crate) fn media_recovering(&self) -> bool {
        self.direct_output
            .live_edge
            .lock()
            .map_or(true, |edge| edge.recovering())
    }

    pub(crate) fn reset_media_clock(&self) {
        if let Ok(mut edge) = self.direct_output.live_edge.lock() {
            *edge = Default::default();
        }
    }

    pub(crate) fn observe_media(&self, ts: u32, seq: u16, received: Instant, now: Instant) -> bool {
        self.direct_output
            .live_edge
            .lock()
            .is_ok_and(|mut edge| edge.observe(ts, seq, received, now))
    }

    pub(crate) fn poll_media(&self, now: Instant) -> bool {
        self.direct_output
            .live_edge
            .lock()
            .is_ok_and(|mut edge| edge.poll(now))
    }

    pub(crate) fn media_ingress_useful(&self, ts: u32, now: Instant) -> bool {
        self.direct_output
            .live_edge
            .lock()
            .is_ok_and(|edge| edge.ingress_useful(ts, now))
    }

    pub(crate) fn discard_queued(&self) {
        while let Ok(old) = self.queued_access_units.try_recv() {
            drop(old);
        }
        metrics::METRICS.au_queue_depth.store(0, Ordering::Relaxed);
    }

    /// Caller must supply an intact, size-checked IDR with its SPS/PPS. Never
    /// use this for a P picture: the retained suffix must be independently decodable.
    pub(crate) fn submit_refresh_access_unit(
        &self,
        data: Vec<u8>,
        received_at: Instant,
        timestamp: u32,
    ) -> SubmitResult {
        if self
            .thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
        {
            return SubmitResult::Disconnected;
        }
        // Reject an impossible allocation before invalidating the current chain.
        if data.len() > super::policy::AU_QUEUE_BYTES {
            return SubmitResult::QueueFull;
        }
        self.begin_resync();
        let mut dropped = 0;
        while let Ok(old) = self.queued_access_units.try_recv() {
            drop(old); // releases its byte reservation
            dropped += 1;
        }
        metrics::METRICS.au_queue_depth.store(0, Ordering::Relaxed);
        super::trace::record("keyframe_cutover_units", timestamp, dropped);
        // Only this producer admits AUs, so a full old queue cannot reject its
        // own replacement. The in-flight old call is rejected by epoch at output.
        self.submit_access_unit(data, received_at, timestamp)
    }

    pub(crate) fn take_recovery_request(&self) -> bool {
        let requested_epoch = self.recovery_needed.swap(0, Ordering::AcqRel);
        requested_epoch != 0 && requested_epoch == self.generation.load(Ordering::Acquire)
    }

    pub fn begin_resync(&self) {
        let epoch = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.direct_output.invalidate_before_epoch(epoch);
        metrics::METRICS.resyncs.fetch_add(1, Ordering::Relaxed);
    }
}

impl Drop for VideoDecodeWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(target_os = "vita")]
fn pin_decoder_thread() {
    // SAFETY: query the calling thread; this SDK call takes no pointers.
    let thread_id = unsafe { vitasdk_sys::sceKernelGetThreadId() };
    // SAFETY: current live thread ID and fixed user-CPU mask; no ownership transfer.
    let result = unsafe {
        vitasdk_sys::sceKernelChangeThreadCpuAffinityMask(
            thread_id,
            vitasdk_sys::SCE_KERNEL_CPU_MASK_USER_2 as i32,
        )
    };
    if result < 0 {
        eprintln!("Failed to pin video decoder thread to user CPU 2: {result:#x}");
    }
}

fn run_decode_loop(
    access_units: Receiver<QueuedAccessUnit>,
    commands: Receiver<DecoderCommand>,
    initial_decoder: HwVideoDecoder,
    config: DecoderConfig,
    state: DecodeState,
) {
    let mut decoder = Some(initial_decoder);
    let mut drain_pending = false;
    let mut drain_enabled = true;

    loop {
        // Check stop between every hardware call, including a long buffered-output run.
        match commands.try_recv() {
            Ok(DecoderCommand::Stop) | Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
        if drain_enabled && drain_pending {
            let pending = decoder
                .as_ref()
                .map_or(0, HwVideoDecoder::pending_output_count);
            if !poll_before_input(access_units.len(), pending)
                && let Ok(access_unit) = access_units.try_recv()
            {
                metrics::METRICS
                    .au_queue_depth
                    .store(access_units.len() as u64, Ordering::Relaxed);
                drain_pending |=
                    decode_queued_access_unit(&mut decoder, config, &state, access_unit);
                continue;
            }
            match poll_decoder(&mut decoder, &state) {
                Ok(more) => drain_pending = more,
                Err(()) => {
                    drain_enabled = false;
                    drain_pending = false;
                }
            }
            // Alternate one output-only call with queued input. Do not hold the
            // texture mutex across calls, or starve input while catching up.
            match access_units.try_recv() {
                Ok(access_unit) => {
                    metrics::METRICS
                        .au_queue_depth
                        .store(access_units.len() as u64, Ordering::Relaxed);
                    drain_pending |=
                        decode_queued_access_unit(&mut decoder, config, &state, access_unit);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
            // Old-epoch pictures still count as drained. Continue even while
            // RTP recovery is waiting for an IDR and there is no new input.
            if drain_enabled && drain_pending {
                continue;
            }
        }
        select_biased! {
            recv(commands) -> command => match command {
                Ok(DecoderCommand::Stop) | Err(_) => break,
            },
            recv(access_units) -> access_unit => {
                let Ok(access_unit) = access_unit else { break };
                metrics::METRICS.au_queue_depth.store(access_units.len() as u64, Ordering::Relaxed);
                drain_pending |= decode_queued_access_unit(
                    &mut decoder,
                    config,
                    &state,
                    access_unit,
                );
            }
        }
    }
}

fn decode_queued_access_unit(
    decoder: &mut Option<HwVideoDecoder>,
    config: DecoderConfig,
    state: &DecodeState,
    mut access_unit: QueuedAccessUnit,
) -> bool {
    let DecodeState {
        generation,
        recovery_needed,
        latest_result,
        result_ready,
        direct_output,
    } = state;
    // Release queue memory credit on dequeue, even for stale generations.
    drop(access_unit.reservation.take());
    if access_unit.generation != generation.load(Ordering::Acquire) {
        super::trace::record("generation_drop", access_unit.rtp_timestamp, 0);
        metrics::METRICS
            .stale_generation
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }

    // A short Cloud burst is valid reference work. Once the entire local budget
    // is exhausted, abandon the epoch once and reacquire a complete IDR; never
    // drop a P picture and feed its dependants as though nothing happened.
    if access_unit.received_at.elapsed() > super::policy::MAX_LOCAL_VIDEO_AGE
        || !direct_output.media_useful(access_unit.rtp_timestamp, Instant::now())
    {
        let epoch = access_unit.generation + 1;
        if generation
            .compare_exchange(
                access_unit.generation,
                epoch,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false; // A concurrently admitted replacement IDR owns recovery.
        }
        direct_output.invalidate_before_epoch(epoch);
        recovery_needed.store(epoch, Ordering::Release);
        metrics::METRICS
            .expired_access_unit
            .fetch_add(1, Ordering::Relaxed);
        super::trace::record(
            "au_deadline_recovery",
            access_unit.rtp_timestamp,
            access_unit.received_at.elapsed().as_micros() as u64,
        );
        result_ready.notify_one();
        return false;
    }

    // Existing damage recovery also rejects dependants until an intact IDR.
    if recovery_needed.load(Ordering::Acquire) == access_unit.generation
        && access_unit.generation != 0
    {
        super::trace::record("generation_drop", access_unit.rtp_timestamp, 0);
        metrics::METRICS
            .stale_generation
            .fetch_add(1, Ordering::Relaxed);
        return false;
    }

    if decoder.is_none() {
        match HwVideoDecoder::new(config) {
            Ok(new_decoder) => *decoder = Some(new_decoder),
            Err(error) => {
                let epoch = generation.fetch_add(1, Ordering::AcqRel) + 1;
                direct_output.invalidate_before_epoch(epoch);
                recovery_needed.store(epoch, Ordering::Release);
                metrics::METRICS
                    .decoder_unavailable
                    .fetch_add(1, Ordering::Relaxed);
                publish_result(
                    latest_result,
                    result_ready,
                    Err(format!("failed to recreate H264 decoder: {error:#}")),
                );
                return false;
            }
        }
    }

    let Some(direct_target) = direct_output.lock_decode_target() else {
        // Do not decode until the renderer has registered stable CDRAM output buffers.
        metrics::METRICS.skipped.fetch_add(1, Ordering::Relaxed);
        let epoch = generation.fetch_add(1, Ordering::AcqRel) + 1;
        direct_output.invalidate_before_epoch(epoch);
        recovery_needed.store(epoch, Ordering::Release);
        result_ready.notify_one();
        return false;
    };
    let age_us = access_unit.queued_at.elapsed().as_micros() as u64;
    super::trace::record("au_queue_wait_us", access_unit.rtp_timestamp, age_us);
    if access_unit.queued_at.elapsed() > AU_PRESSURE_AGE {
        super::trace::record("au_queue_pressure_us", access_unit.rtp_timestamp, age_us);
    }
    metrics::METRICS
        .au_age_sum_us
        .fetch_add(age_us, Ordering::Relaxed);
    metrics::METRICS
        .au_age_count
        .fetch_add(1, Ordering::Relaxed);
    metrics::METRICS
        .au_age_max_us
        .fetch_max(age_us, Ordering::Relaxed);
    // Measure the hardware call and contain an unexpected decoder panic inside its worker.
    metrics::METRICS
        .decode_calls
        .fetch_add(1, Ordering::Relaxed);
    let decode_started_at = Instant::now();
    super::trace::record(
        "decode_submit",
        access_unit.rtp_timestamp,
        access_unit.received_at.elapsed().as_micros() as u64,
    );
    let decode_result = catch_unwind(AssertUnwindSafe(|| {
        decoder.as_mut().expect("decoder recreated above").decode(
            &access_unit.data,
            direct_target.target,
            access_unit.rtp_timestamp,
            access_unit.received_at,
            decode_started_at,
            access_unit.generation,
        )
    }));
    let decode_us = decode_started_at.elapsed().as_micros() as u64;
    super::trace::record("decode_return", access_unit.rtp_timestamp, decode_us);
    metrics::METRICS
        .decode_us
        .store(decode_us, Ordering::Relaxed);
    metrics::METRICS
        .decode_sum_us
        .fetch_add(decode_us, Ordering::Relaxed);
    metrics::METRICS
        .decode_count
        .fetch_add(1, Ordering::Relaxed);
    metrics::METRICS
        .decode_max_us
        .fetch_max(decode_us, Ordering::Relaxed);
    handle_decode_result(
        decoder,
        state,
        Some(&access_unit),
        access_unit.generation,
        direct_target,
        decode_result,
    );
    super::trace::record(
        "decoder_pending",
        access_unit.rtp_timestamp,
        decoder
            .as_ref()
            .map_or(0, HwVideoDecoder::pending_output_count) as u64,
    );
    decoder
        .as_ref()
        .is_some_and(HwVideoDecoder::has_pending_output)
}

/// Return true when a picture was removed, even if its epoch prevents display.
fn handle_decode_result(
    decoder: &mut Option<HwVideoDecoder>,
    state: &DecodeState,
    input: Option<&QueuedAccessUnit>,
    call_generation: u64,
    direct_target: DirectVideoTargetGuard<'_>,
    decode_result: std::thread::Result<Result<Option<DecodedPicture>>>,
) -> bool {
    let DecodeState {
        generation,
        recovery_needed,
        latest_result,
        result_ready,
        ..
    } = state;
    match decode_result {
        Ok(Ok(Some(picture))) => {
            if call_generation != generation.load(Ordering::Acquire) {
                metrics::METRICS
                    .stale_generation
                    .fetch_add(1, Ordering::Relaxed);
                return true;
            }
            if let Some(timing) = picture.timing {
                if timing.epoch != generation.load(Ordering::Acquire) {
                    metrics::METRICS
                        .stale_generation
                        .fetch_add(1, Ordering::Relaxed);
                    super::trace::record("old_picture_epoch", timing.rtp_timestamp, timing.epoch);
                    return true;
                }
                let decoder_age_us = timing
                    .decoded_at
                    .saturating_duration_since(timing.submitted_at)
                    .as_micros() as u64;
                metrics::METRICS
                    .decoder_age_sum_us
                    .fetch_add(decoder_age_us, Ordering::Relaxed);
                metrics::METRICS
                    .decoder_age_count
                    .fetch_add(1, Ordering::Relaxed);
                metrics::METRICS
                    .decoder_age_max_us
                    .fetch_max(decoder_age_us, Ordering::Relaxed);
                super::trace::record("decoder_residence_us", timing.rtp_timestamp, decoder_age_us);
                metrics::METRICS
                    .output_pts_matched
                    .fetch_add(1, Ordering::Relaxed);
                let age_us = timing
                    .decoded_at
                    .saturating_duration_since(timing.received_at)
                    .as_micros() as u64;
                super::trace::record("picture_output_rtp", timing.rtp_timestamp, age_us);
            } else {
                metrics::METRICS
                    .output_pts_unmatched
                    .fetch_add(1, Ordering::Relaxed);
                // Decode can return an older queued picture on an input call
                // too. The submitted AU's epoch never proves output identity.
                super::trace::record("unknown_picture_pts", 0, 0);
                return true;
            }
            let output_rtp = picture.timing.map(|t| t.rtp_timestamp).unwrap_or(0);
            let output_age = picture
                .timing
                .map(|t| t.received_at.elapsed().as_micros() as u64)
                .unwrap_or(0);
            super::trace::record("picture_produced_output", output_rtp, output_age);
            metrics::METRICS.decoded.fetch_add(1, Ordering::Relaxed);
            let Some((_texture_index, generation)) = direct_target.publish(picture.timing) else {
                return true; // Hardware work retired, but this picture is obsolete.
            };
            super::trace::record("picture_generation_output", output_rtp, generation);
            if let Some(input) = input {
                metrics::METRICS.pipeline_age_us.store(
                    input.queued_at.elapsed().as_micros() as u64,
                    Ordering::Relaxed,
                );
            }
            publish_result(latest_result, result_ready, Ok(DecodedFrame));
            true
        }
        Ok(Ok(None)) => {
            if let Some(input) = input {
                super::trace::record("no_picture", input.rtp_timestamp, 0);
                metrics::METRICS.no_picture.fetch_add(1, Ordering::Relaxed);
            }
            false
        }
        error => {
            // Only input decode errors use the existing recovery behavior.
            // poll_decoder handles an unsupported output-only call separately.
            let message = match error {
                Ok(Err(error)) => error.to_string(),
                Err(_) => "H264 decoder panicked and was restarted".to_owned(),
                _ => unreachable!(),
            };
            metrics::METRICS.resets.fetch_add(1, Ordering::Relaxed);
            *decoder = None;
            let epoch = generation.fetch_add(1, Ordering::AcqRel) + 1;
            state.direct_output.invalidate_before_epoch(epoch);
            recovery_needed.store(epoch, Ordering::Release);
            publish_result(latest_result, result_ready, Err(message));
            false
        }
    }
}

fn poll_decoder(decoder: &mut Option<HwVideoDecoder>, state: &DecodeState) -> Result<bool, ()> {
    let DecodeState {
        generation,
        latest_result,
        result_ready,
        direct_output,
        ..
    } = state;
    let Some(hw) = decoder.as_mut() else {
        return Ok(false);
    };
    // Once every admitted PTS has returned there is nothing to ask the firmware
    // for. Avoid an empty hardware call in the ordinary one-input/one-output case.
    if !hw.has_pending_output() {
        return Ok(false);
    }
    let Some(target) = direct_output.lock_decode_target() else {
        return Ok(false);
    };
    let call_generation = generation.load(Ordering::Acquire);
    let started = Instant::now();
    let result = catch_unwind(AssertUnwindSafe(|| hw.poll(target.target)));
    super::trace::record(
        "decoder_poll_return",
        0,
        started.elapsed().as_micros() as u64,
    );
    metrics::METRICS
        .decoder_poll_calls
        .fetch_add(1, Ordering::Relaxed);
    if !matches!(&result, Ok(Ok(_))) {
        // Fail visibly once, not a reset/reconnect loop if a firmware rejects
        // empty-input decode. Preserve its state and disable polling this session.
        metrics::METRICS
            .decoder_poll_failed
            .fetch_add(1, Ordering::Relaxed);
        super::trace::record("decoder_poll_failed", 0, 1);
        let message = match result {
            Ok(Err(error)) => format!("AVC output polling disabled for this session: {error:#}"),
            _ => "AVC output polling panicked; disabled for this session".to_owned(),
        };
        eprintln!("{message}");
        publish_result(latest_result, result_ready, Err(message));
        return Err(());
    }
    if matches!(&result, Ok(Ok(Some(_)))) {
        metrics::METRICS
            .decoder_poll_pictures
            .fetch_add(1, Ordering::Relaxed);
    }
    Ok(handle_decode_result(
        decoder,
        state,
        None,
        call_generation,
        target,
        result,
    ))
}

fn publish_result(
    slot: &Mutex<Option<DecodeResult>>,
    result_ready: &tokio::sync::Notify,
    result: DecodeResult,
) {
    if let Ok(mut latest) = slot.lock()
        && latest.replace(result).is_some()
    {
        metrics::METRICS.replaced.fetch_add(1, Ordering::Relaxed);
    }
    result_ready.notify_one();
}
