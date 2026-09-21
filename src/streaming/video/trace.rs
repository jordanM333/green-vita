//! Bounded, metadata-only flight recorder. No media payloads, URLs or tokens.
//! Timestamps identify submissions, not guaranteed AVC output PTS: a no-picture
//! decoder call means hardware output association must not be assumed exact.
//! The explicit picture_output_rtp / receive_to_gpu_done_us / frame_feedback
//! stages instead carry a verified output RTP identity. decoder_output_pts uses
//! the submission RTP column and stores the decoder's raw 90 kHz PTS in value.
//! decoder_poll_output_pts has no submission; its value is the returned PTS.
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

const CAPACITY: usize = 4096;
// Retain at least 35 minutes of 1 Hz history for early/end acceptance checks.
// Approximately 4 MiB with the current status format; the per-event ring stays bounded.
const HISTORY_CAPACITY: usize = 2100;
static TRACE: Mutex<Option<Trace>> = Mutex::new(None);
struct Trace {
    start: Instant,
    events: VecDeque<(u128, &'static str, u32, u64)>,
    history: VecDeque<(u128, String)>,
}

pub(crate) fn reset() {
    if let Ok(mut trace) = TRACE.lock() {
        *trace = Some(Trace { start: Instant::now(), events: VecDeque::with_capacity(CAPACITY),
            history: VecDeque::with_capacity(HISTORY_CAPACITY) });
    }
}

pub(crate) fn record(stage: &'static str, timestamp: u32, value: u64) {
    if let Ok(mut trace) = TRACE.lock() && let Some(trace) = trace.as_mut() {
        if trace.events.len() == CAPACITY { trace.events.pop_front(); }
        trace.events.push_back((trace.start.elapsed().as_micros(), stage, timestamp, value));
    }
}

/// Called with the once-per-second diagnostics, preserving gradual changes.
pub(crate) fn status_snapshot(status: &str) {
    if let Ok(mut trace) = TRACE.lock() && let Some(trace) = trace.as_mut() {
        let elapsed = trace.start.elapsed().as_millis();
        if trace.history.back().is_some_and(|(time, _)| elapsed - time < 1000) { return; }
        if trace.history.len() == HISTORY_CAPACITY { trace.history.pop_front(); }
        trace.history.push_back((elapsed, status.to_owned()));
    }
}

/// Called after streaming stops; no filesystem writes in the media loop.
pub(crate) fn save(status: &str) {
    let (text, history) = if let Ok(trace) = TRACE.lock() && let Some(trace) = trace.as_ref() {
        let mut text = String::from("elapsed_us,stage,submission_rtp_timestamp,value_us_or_reason\n");
        for (time, stage, timestamp, value) in &trace.events {
            use std::fmt::Write;
            let _ = writeln!(text, "{time},{stage},{timestamp},{value}");
        }
        let mut history = String::new();
        for (time, status) in &trace.history {
            use std::fmt::Write;
            let _ = writeln!(history, "elapsed_ms={time}\n{status}\n");
        }
        (text, history)
    } else { return; };
    let dir = "ux0:data/green-vita-540-test";
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        // Retain one prior session so a refresh does not erase its evidence.
        for name in ["trace.csv", "status.txt", "history.txt"] {
            let current = format!("{dir}/pipeline-{name}");
            let previous = format!("{dir}/pipeline-previous-{name}");
            if std::path::Path::new(&current).exists() {
                if std::path::Path::new(&previous).exists() { std::fs::remove_file(&previous)?; }
                std::fs::rename(current, previous)?;
            }
        }
        std::fs::write(format!("{dir}/pipeline-trace.csv"), text)?;
        std::fs::write(format!("{dir}/pipeline-history.txt"), history)?;
        std::fs::write(format!("{dir}/pipeline-status.txt"), status)
    })();
    if let Err(error) = result { eprintln!("Could not save pipeline trace: {error}"); }
}
