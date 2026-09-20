//! Bounded, metadata-only flight recorder. No media payloads, URLs or tokens.
//! Timestamps identify submissions, not guaranteed AVC output PTS: a no-picture
//! decoder call means hardware output association must not be assumed exact.
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

const CAPACITY: usize = 4096;
static TRACE: Mutex<Option<Trace>> = Mutex::new(None);
struct Trace {
    start: Instant,
    events: VecDeque<(u128, &'static str, u32, u64)>,
}

pub(crate) fn reset() {
    if let Ok(mut trace) = TRACE.lock() {
        *trace = Some(Trace { start: Instant::now(), events: VecDeque::with_capacity(CAPACITY) });
    }
}

pub(crate) fn record(stage: &'static str, timestamp: u32, value: u64) {
    if let Ok(mut trace) = TRACE.lock() && let Some(trace) = trace.as_mut() {
        if trace.events.len() == CAPACITY { trace.events.pop_front(); }
        trace.events.push_back((trace.start.elapsed().as_micros(), stage, timestamp, value));
    }
}

/// Called after streaming stops; no filesystem writes in the media loop.
pub(crate) fn save(status: &str) {
    let text = if let Ok(trace) = TRACE.lock() && let Some(trace) = trace.as_ref() {
        let mut text = String::from("elapsed_us,stage,submission_rtp_timestamp,value_us_or_reason\n");
        for (time, stage, timestamp, value) in &trace.events {
            use std::fmt::Write;
            let _ = writeln!(text, "{time},{stage},{timestamp},{value}");
        }
        text
    } else { return; };
    let dir = "ux0:data/green-vita-540-test";
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(format!("{dir}/pipeline-trace.csv"), text)?;
        std::fs::write(format!("{dir}/pipeline-status.txt"), status)
    })();
    if let Err(error) = result { eprintln!("Could not save pipeline trace: {error}"); }
}
