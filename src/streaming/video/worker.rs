use super::decoder::HwVideoDecoder;
use super::metrics;
use super::{DecodedFrame, DecoderConfig, DirectVideoOutput};
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::policy::{AU_QUEUE_CAPACITY, expired};

struct QueuedAccessUnit {
    data: Vec<u8>,
    queued_at: Instant,
    received_at: Instant,
    rtp_timestamp: u32,
    generation: u64,
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

pub struct VideoDecodeWorker {
    thread: Option<std::thread::JoinHandle<()>>,
    access_units: Sender<QueuedAccessUnit>,
    commands: Sender<DecoderCommand>,
    generation: Arc<AtomicU64>,
    recovery_needed: Arc<AtomicBool>,
    pub(crate) latest_result: Arc<Mutex<Option<DecodeResult>>>,
    pub(crate) result_ready: Arc<tokio::sync::Notify>,
}

impl VideoDecodeWorker {
    pub fn spawn(config: DecoderConfig, direct_output: Arc<DirectVideoOutput>) -> Result<Self> {
        let decoder =
            HwVideoDecoder::new(config).context("failed to create hardware H264 decoder")?;
        direct_output.decoder_ready.store(true, Ordering::Release);
        let (access_units, worker_access_units) = bounded(AU_QUEUE_CAPACITY);
        let (commands, worker_commands) = bounded(1);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let recovery_needed = Arc::new(AtomicBool::new(false));
        let worker_recovery_needed = Arc::clone(&recovery_needed);
        let latest_result = Arc::new(Mutex::new(None));
        let worker_latest_result = Arc::clone(&latest_result);
        let result_ready = Arc::new(tokio::sync::Notify::new());
        let worker_result_ready = Arc::clone(&result_ready);
        let worker_direct_output = Arc::clone(&direct_output);

        let ready_output = Arc::clone(&direct_output);
        let thread = std::thread::Builder::new()
            .name("green-vita-video-decode".to_owned())
            .spawn(move || {
                #[cfg(target_os = "vita")]
                pin_decoder_thread();
                run_decode_loop(
                    worker_access_units,
                    worker_commands,
                    worker_generation,
                    worker_recovery_needed,
                    worker_latest_result,
                    worker_result_ready,
                    decoder,
                    config,
                    worker_direct_output,
                );
                ready_output.decoder_ready.store(false, Ordering::Release);
            })
            .context("failed to spawn video decode worker")?;

        Ok(Self {
            thread: Some(thread),
            access_units,
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
            if thread.join().is_err() { eprintln!("Video decode worker panicked during shutdown"); }
        }
    }

    pub fn submit_access_unit(
        &self,
        data: Vec<u8>,
        first_packet_at: Instant,
        rtp_timestamp: u32,
    ) -> SubmitResult {
        let access_unit = QueuedAccessUnit {
            data,
            queued_at: Instant::now(),
            received_at: first_packet_at,
            rtp_timestamp,
            generation: self.generation.load(Ordering::Acquire),
        };
        match self.access_units.try_send(access_unit) {
            Ok(()) => {
                let depth = self.access_units.len() as u64;
                metrics::METRICS.au_queue_depth.store(depth, Ordering::Relaxed);
                metrics::METRICS.au_queue_max.fetch_max(depth, Ordering::Relaxed);
                SubmitResult::Submitted
            }
            Err(TrySendError::Full(_)) => {
                metrics::METRICS.queue_full.fetch_add(1, Ordering::Relaxed);
                SubmitResult::QueueFull
            }
            Err(TrySendError::Disconnected(_)) => SubmitResult::Disconnected,
        }
    }

    pub(crate) fn take_recovery_request(&self) -> bool {
        self.recovery_needed.swap(false, Ordering::AcqRel)
    }

    pub fn begin_resync(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
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
    let thread_id = unsafe { vitasdk_sys::sceKernelGetThreadId() };
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
    generation: Arc<AtomicU64>,
    recovery_needed: Arc<AtomicBool>,
    latest_result: Arc<Mutex<Option<DecodeResult>>>,
    result_ready: Arc<tokio::sync::Notify>,
    initial_decoder: HwVideoDecoder,
    config: DecoderConfig,
    direct_output: Arc<DirectVideoOutput>,
) {
    let mut decoder = Some(initial_decoder);

    loop {
        select_biased! {
            recv(commands) -> command => match command {
                Ok(DecoderCommand::Stop) | Err(_) => break,
            },
            recv(access_units) -> access_unit => {
                let Ok(access_unit) = access_unit else { break };
                metrics::METRICS.au_queue_depth.store(access_units.len() as u64, Ordering::Relaxed);
                decode_queued_access_unit(
                    &mut decoder,
                    config,
                    &generation,
                    &recovery_needed,
                    &latest_result,
                    &result_ready,
                    access_unit,
                    &direct_output,
                );
            }
        }
    }
}

fn decode_queued_access_unit(
    decoder: &mut Option<HwVideoDecoder>,
    config: DecoderConfig,
    generation: &AtomicU64,
    recovery_needed: &AtomicBool,
    latest_result: &Mutex<Option<DecodeResult>>,
    result_ready: &tokio::sync::Notify,
    access_unit: QueuedAccessUnit,
    direct_output: &DirectVideoOutput,
) {
    if access_unit.generation != generation.load(Ordering::Acquire) {
        super::trace::record("generation_drop", access_unit.rtp_timestamp, 0);
        metrics::METRICS.stale_generation.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // Never keep playing an old compressed backlog. Losing a reference picture
    // requires an IDR, not arbitrary P-frame replacement or a decoder reset.
    if recovery_needed.load(Ordering::Acquire)
        || expired(access_unit.queued_at, Instant::now())
    {
        super::trace::record("age_drop", access_unit.rtp_timestamp,
            access_unit.queued_at.elapsed().as_micros() as u64);
        metrics::METRICS.stale_generation.fetch_add(1, Ordering::Relaxed);
        generation.fetch_add(1, Ordering::AcqRel);
        recovery_needed.store(true, Ordering::Release);
        result_ready.notify_one();
        return;
    }

    if decoder.is_none() {
        match HwVideoDecoder::new(config) {
            Ok(new_decoder) => *decoder = Some(new_decoder),
            Err(error) => {
                generation.fetch_add(1, Ordering::AcqRel);
                recovery_needed.store(true, Ordering::Release);
                metrics::METRICS.decoder_unavailable.fetch_add(1, Ordering::Relaxed);
                publish_result(
                    latest_result,
                    result_ready,
                    Err(format!("failed to recreate H264 decoder: {error:#}")),
                );
                return;
            }
        }
    }

    let Some(direct_target) = direct_output.lock_decode_target() else {
        // Do not decode until the renderer has registered stable CDRAM output buffers.
        metrics::METRICS.skipped.fetch_add(1, Ordering::Relaxed);
        generation.fetch_add(1, Ordering::AcqRel);
        recovery_needed.store(true, Ordering::Release);
        result_ready.notify_one();
        return;
    };
    let age_us = access_unit.queued_at.elapsed().as_micros() as u64;
    // Acquiring an output surface can itself wait behind the renderer. Recheck
    // after acquiring it so the queue-age limit also covers that wait.
    if expired(access_unit.queued_at, Instant::now()) {
        super::trace::record("output_wait_expired", access_unit.rtp_timestamp, age_us);
        metrics::METRICS.stale_generation.fetch_add(1, Ordering::Relaxed);
        generation.fetch_add(1, Ordering::AcqRel);
        recovery_needed.store(true, Ordering::Release);
        result_ready.notify_one();
        return;
    }
    metrics::METRICS.au_age_sum_us.fetch_add(age_us, Ordering::Relaxed);
    metrics::METRICS.au_age_count.fetch_add(1, Ordering::Relaxed);
    metrics::METRICS.au_age_max_us.fetch_max(age_us, Ordering::Relaxed);
    // Measure the hardware call and contain an unexpected decoder panic inside its worker.
    metrics::METRICS.decode_calls.fetch_add(1, Ordering::Relaxed);
    let decode_started_at = Instant::now();
    super::trace::record("decode_submit", access_unit.rtp_timestamp,
        access_unit.received_at.elapsed().as_micros() as u64);
    let decode_result = catch_unwind(AssertUnwindSafe(|| {
        decoder
            .as_mut()
            .expect("decoder recreated above")
            .decode(&access_unit.data, direct_target.target, access_unit.rtp_timestamp,
                access_unit.received_at, decode_started_at, access_unit.generation)
    }));
    let decode_us = decode_started_at.elapsed().as_micros() as u64;
    super::trace::record("decode_return", access_unit.rtp_timestamp, decode_us);
    metrics::METRICS.decode_us.store(decode_us, Ordering::Relaxed);
    metrics::METRICS.decode_sum_us.fetch_add(decode_us, Ordering::Relaxed);
    metrics::METRICS.decode_count.fetch_add(1, Ordering::Relaxed);
    metrics::METRICS.decode_max_us.fetch_max(decode_us, Ordering::Relaxed);
    if access_unit.generation != generation.load(Ordering::Acquire) {
        metrics::METRICS.stale_generation.fetch_add(1, Ordering::Relaxed);
        return;
    }

    match decode_result {
        Ok(Ok(Some(picture))) => {
            if let Some(timing) = picture.timing {
                if timing.epoch != generation.load(Ordering::Acquire) {
                    metrics::METRICS.stale_generation.fetch_add(1, Ordering::Relaxed);
                    super::trace::record("old_picture_epoch", timing.rtp_timestamp, timing.epoch);
                    return;
                }
                metrics::METRICS.output_pts_matched.fetch_add(1, Ordering::Relaxed);
                let age_us = timing.decoded_at.saturating_duration_since(timing.received_at).as_micros() as u64;
                super::trace::record("picture_output_rtp", timing.rtp_timestamp, age_us);
            } else {
                metrics::METRICS.output_pts_unmatched.fetch_add(1, Ordering::Relaxed);
            }
            super::trace::record("picture_produced", access_unit.rtp_timestamp,
                access_unit.received_at.elapsed().as_micros() as u64);
            metrics::METRICS.decoded.fetch_add(1, Ordering::Relaxed);
            let (texture_index, generation) = direct_target.publish(picture.timing);
            super::trace::record("picture_generation", access_unit.rtp_timestamp, generation);
            metrics::METRICS.pipeline_age_us.store(
                access_unit.queued_at.elapsed().as_micros() as u64,
                Ordering::Relaxed,
            );
            publish_result(
                latest_result,
                result_ready,
                Ok(DecodedFrame {
                    texture_index,
                    generation,
                }),
            );
        }
        Ok(Ok(None)) => {
            super::trace::record("no_picture", access_unit.rtp_timestamp, 0);
            metrics::METRICS.no_picture.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Err(error)) => {
            metrics::METRICS.resets.fetch_add(1, Ordering::Relaxed);
            *decoder = None;
            generation.fetch_add(1, Ordering::AcqRel);
            recovery_needed.store(true, Ordering::Release);
            publish_result(latest_result, result_ready, Err(error.to_string()));
        }
        Err(_) => {
            metrics::METRICS.resets.fetch_add(1, Ordering::Relaxed);
            eprintln!("H264 decoder panicked; recreating decoder on next frame");
            *decoder = None;
            generation.fetch_add(1, Ordering::AcqRel);
            recovery_needed.store(true, Ordering::Release);
            publish_result(
                latest_result,
                result_ready,
                Err("H264 decoder panicked and was restarted".to_owned()),
            );
        }
    }
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
