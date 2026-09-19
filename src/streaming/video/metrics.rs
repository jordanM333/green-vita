use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct VideoMetrics {
    pub(crate) rtp_assembly_sum_us: AtomicU64,
    pub(crate) rtp_assembly_count: AtomicU64,
    pub(crate) rtp_assembly_max_us: AtomicU64,
    pub(crate) decode_us: AtomicU64,
    pub(crate) decode_sum_us: AtomicU64,
    pub(crate) decode_count: AtomicU64,
    pub(crate) decode_max_us: AtomicU64,
    pub(crate) pipeline_age_us: AtomicU64,
    pub(crate) decode_calls: AtomicU64,
    pub(crate) no_picture: AtomicU64,
    pub(crate) picture_dimensions: AtomicU64,
    pub(crate) au_queue_depth: AtomicU64,
    pub(crate) au_queue_max: AtomicU64,
    pub(crate) au_age_sum_us: AtomicU64,
    pub(crate) au_age_count: AtomicU64,
    pub(crate) au_age_max_us: AtomicU64,
    pub(crate) paint_sum_us: AtomicU64,
    pub(crate) paint_count: AtomicU64,
    pub(crate) paint_max_us: AtomicU64,
    pub(crate) ui_loop_sum_us: AtomicU64,
    pub(crate) ui_loop_count: AtomicU64,
    pub(crate) ui_loop_max_us: AtomicU64,
    pub(crate) texture_superseded: AtomicU64,
    pub(crate) display_age_sum_us: AtomicU64,
    pub(crate) display_age_count: AtomicU64,
    pub(crate) display_age_max_us: AtomicU64,
    pub(crate) handoff_replaced: AtomicU64,
    pub(crate) stale_generation: AtomicU64,
    pub(crate) decoder_unavailable: AtomicU64,
    pub(crate) decoded: AtomicU64,
    pub(crate) skipped: AtomicU64,
    pub(crate) presented: AtomicU64,
    pub(crate) replaced: AtomicU64,
    pub(crate) queue_full: AtomicU64,
    pub(crate) resyncs: AtomicU64,
    pub(crate) resets: AtomicU64,
    pub(crate) audio_sdl_queue_ms: AtomicU64,
    pub(crate) audio_opus_pending: AtomicU64,
    pub(crate) audio_pcm_pending: AtomicU64,
    pub(crate) audio_underruns: AtomicU64,
    pub(crate) audio_queue_resets: AtomicU64,
    pub(crate) audio_latency_trims: AtomicU64,
    pub(crate) audio_pcm_discarded: AtomicU64,
    pub(crate) audio_opus_dropped: AtomicU64,
    pub(crate) audio_batch_dropped: AtomicU64,
    pub(crate) audio_batch_age_sum_us: AtomicU64,
    pub(crate) audio_batch_age_count: AtomicU64,
    pub(crate) audio_batch_age_max_us: AtomicU64,
    pub(crate) input_send_age_sum_us: AtomicU64,
    pub(crate) input_send_age_count: AtomicU64,
    pub(crate) input_send_age_max_us: AtomicU64,
    pub(crate) rtc_pump_sum_us: AtomicU64,
    pub(crate) rtc_pump_count: AtomicU64,
    pub(crate) rtc_pump_max_us: AtomicU64,
}

pub(crate) static METRICS: VideoMetrics = VideoMetrics {
    rtp_assembly_sum_us: AtomicU64::new(0),
    rtp_assembly_count: AtomicU64::new(0),
    rtp_assembly_max_us: AtomicU64::new(0),
    decode_us: AtomicU64::new(0),
    decode_sum_us: AtomicU64::new(0),
    decode_count: AtomicU64::new(0),
    decode_max_us: AtomicU64::new(0),
    pipeline_age_us: AtomicU64::new(0),
    decode_calls: AtomicU64::new(0),
    no_picture: AtomicU64::new(0),
    picture_dimensions: AtomicU64::new(0),
    au_queue_depth: AtomicU64::new(0),
    au_queue_max: AtomicU64::new(0),
    au_age_sum_us: AtomicU64::new(0),
    au_age_count: AtomicU64::new(0),
    au_age_max_us: AtomicU64::new(0),
    paint_sum_us: AtomicU64::new(0),
    paint_count: AtomicU64::new(0),
    paint_max_us: AtomicU64::new(0),
    ui_loop_sum_us: AtomicU64::new(0),
    ui_loop_count: AtomicU64::new(0),
    ui_loop_max_us: AtomicU64::new(0),
    texture_superseded: AtomicU64::new(0),
    display_age_sum_us: AtomicU64::new(0),
    display_age_count: AtomicU64::new(0),
    display_age_max_us: AtomicU64::new(0),
    handoff_replaced: AtomicU64::new(0),
    stale_generation: AtomicU64::new(0),
    decoder_unavailable: AtomicU64::new(0),
    decoded: AtomicU64::new(0),
    skipped: AtomicU64::new(0),
    presented: AtomicU64::new(0),
    replaced: AtomicU64::new(0),
    queue_full: AtomicU64::new(0),
    resyncs: AtomicU64::new(0),
    resets: AtomicU64::new(0),
    audio_sdl_queue_ms: AtomicU64::new(0),
    audio_opus_pending: AtomicU64::new(0),
    audio_pcm_pending: AtomicU64::new(0),
    audio_underruns: AtomicU64::new(0),
    audio_queue_resets: AtomicU64::new(0),
    audio_latency_trims: AtomicU64::new(0),
    audio_pcm_discarded: AtomicU64::new(0),
    audio_opus_dropped: AtomicU64::new(0),
    audio_batch_dropped: AtomicU64::new(0),
    audio_batch_age_sum_us: AtomicU64::new(0),
    audio_batch_age_count: AtomicU64::new(0),
    audio_batch_age_max_us: AtomicU64::new(0),
    input_send_age_sum_us: AtomicU64::new(0),
    input_send_age_count: AtomicU64::new(0),
    input_send_age_max_us: AtomicU64::new(0),
    rtc_pump_sum_us: AtomicU64::new(0),
    rtc_pump_count: AtomicU64::new(0),
    rtc_pump_max_us: AtomicU64::new(0),
};

pub fn video_performance_summary() -> String {
    let picture = METRICS.picture_dimensions.load(Ordering::Relaxed);
    let picture_size = if picture == 0 {
        "?".to_owned()
    } else {
        format!("{}x{}", picture >> 32, picture as u32)
    };
    let rtp_sum = METRICS.rtp_assembly_sum_us.swap(0, Ordering::Relaxed);
    let rtp_count = METRICS.rtp_assembly_count.swap(0, Ordering::Relaxed);
    let rtp_average = rtp_sum.checked_div(rtp_count).unwrap_or(0) / 1000;
    let rtp_max = METRICS.rtp_assembly_max_us.swap(0, Ordering::Relaxed) / 1000;
    let au_age_sum = METRICS.au_age_sum_us.swap(0, Ordering::Relaxed);
    let au_age_count = METRICS.au_age_count.swap(0, Ordering::Relaxed);
    let au_age_average = au_age_sum.checked_div(au_age_count).unwrap_or(0) / 1000;
    let au_age_max = METRICS.au_age_max_us.swap(0, Ordering::Relaxed) / 1000;
    let decode_sum = METRICS.decode_sum_us.swap(0, Ordering::Relaxed);
    let decode_count = METRICS.decode_count.swap(0, Ordering::Relaxed);
    let decode_average = decode_sum.checked_div(decode_count).unwrap_or(0) / 1000;
    let decode_max = METRICS.decode_max_us.swap(0, Ordering::Relaxed) / 1000;
    let paint_sum = METRICS.paint_sum_us.swap(0, Ordering::Relaxed);
    let paint_count = METRICS.paint_count.swap(0, Ordering::Relaxed);
    let paint_average = paint_sum.checked_div(paint_count).unwrap_or(0) / 1000;
    let paint_max = METRICS.paint_max_us.swap(0, Ordering::Relaxed) / 1000;
    let ui_loop_sum = METRICS.ui_loop_sum_us.swap(0, Ordering::Relaxed);
    let ui_loop_count = METRICS.ui_loop_count.swap(0, Ordering::Relaxed);
    let ui_loop_average = ui_loop_sum.checked_div(ui_loop_count).unwrap_or(0) / 1000;
    let ui_loop_max = METRICS.ui_loop_max_us.swap(0, Ordering::Relaxed) / 1000;
    let display_age_sum = METRICS.display_age_sum_us.swap(0, Ordering::Relaxed);
    let display_age_count = METRICS.display_age_count.swap(0, Ordering::Relaxed);
    let display_age_average = display_age_sum.checked_div(display_age_count).unwrap_or(0) / 1000;
    let display_age_max = METRICS.display_age_max_us.swap(0, Ordering::Relaxed) / 1000;
    let batch_age_sum = METRICS.audio_batch_age_sum_us.swap(0, Ordering::Relaxed);
    let batch_age_count = METRICS.audio_batch_age_count.swap(0, Ordering::Relaxed);
    let batch_age_avg = batch_age_sum.checked_div(batch_age_count).unwrap_or(0) / 1000;
    let batch_age_max = METRICS.audio_batch_age_max_us.swap(0, Ordering::Relaxed) / 1000;
    let input_age_sum = METRICS.input_send_age_sum_us.swap(0, Ordering::Relaxed);
    let input_age_count = METRICS.input_send_age_count.swap(0, Ordering::Relaxed);
    let input_age_avg = input_age_sum.checked_div(input_age_count).unwrap_or(0) / 1000;
    let input_age_max = METRICS.input_send_age_max_us.swap(0, Ordering::Relaxed) / 1000;
    let rtc_pump_sum = METRICS.rtc_pump_sum_us.swap(0, Ordering::Relaxed);
    let rtc_pump_count = METRICS.rtc_pump_count.swap(0, Ordering::Relaxed);
    let rtc_pump_avg = rtc_pump_sum.checked_div(rtc_pump_count).unwrap_or(0) / 1000;
    let rtc_pump_max = METRICS.rtc_pump_max_us.swap(0, Ordering::Relaxed) / 1000;
    let base = format!(
        "Q depth/max:{}/{} AUage:{au_age_average}/{au_age_max}ms dec:{decode_average}/{decode_max}ms paint:{paint_average}/{paint_max}ms ui:{ui_loop_average}/{ui_loop_max}ms\n\
         FPS hwCall:{} decoded:{} shown:{} ui:{paint_count} pic:{} noPic:{} noOut:{} qFull/s:{} asm:{rtp_average}/{rtp_max}ms\n\
         Stage texRepl:{} showAge:{display_age_average}/{display_age_max}ms staleAU:{} noDec:{} mailRepl:{} handoffRepl:{} resync:{} reset:{}",
        METRICS.au_queue_depth.load(Ordering::Relaxed),
        METRICS.au_queue_max.load(Ordering::Relaxed),
        METRICS.decode_calls.swap(0, Ordering::Relaxed),
        METRICS.decoded.swap(0, Ordering::Relaxed),
        METRICS.presented.swap(0, Ordering::Relaxed),
        picture_size,
        METRICS.no_picture.swap(0, Ordering::Relaxed),
        METRICS.skipped.swap(0, Ordering::Relaxed),
        METRICS.queue_full.swap(0, Ordering::Relaxed),
        METRICS.texture_superseded.load(Ordering::Relaxed),
        METRICS.stale_generation.load(Ordering::Relaxed),
        METRICS.decoder_unavailable.load(Ordering::Relaxed),
        METRICS.replaced.swap(0, Ordering::Relaxed),
        METRICS.handoff_replaced.load(Ordering::Relaxed),
        METRICS.resyncs.load(Ordering::Relaxed),
        METRICS.resets.load(Ordering::Relaxed),
    );
    format!(
        "{base}\nDelay SDL:{}ms opusQ:{} pcmQ:{} batchAge:{batch_age_avg}/{batch_age_max}ms underrun:{} trim:{} skip:{} clr:{} lost:{}\nInput local:{input_age_avg}/{input_age_max}ms RTCpump:{rtc_pump_avg}/{rtc_pump_max}ms",
        METRICS.audio_sdl_queue_ms.load(Ordering::Relaxed),
        METRICS.audio_opus_pending.load(Ordering::Relaxed),
        METRICS.audio_pcm_pending.load(Ordering::Relaxed),
        METRICS.audio_underruns.load(Ordering::Relaxed),
        METRICS.audio_latency_trims.load(Ordering::Relaxed),
        METRICS.audio_pcm_discarded.load(Ordering::Relaxed),
        METRICS.audio_queue_resets.load(Ordering::Relaxed),
        METRICS.audio_opus_dropped.load(Ordering::Relaxed)
            + METRICS.audio_batch_dropped.load(Ordering::Relaxed),
    )
}
