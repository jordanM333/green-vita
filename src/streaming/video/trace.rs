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
const INCIDENT_CAPACITY: usize = 1024;
// Retain at least 35 minutes of 1 Hz history for early/end acceptance checks.
// Approximately 4 MiB with the current status format; the per-event ring stays bounded.
const HISTORY_CAPACITY: usize = 2100;
static TRACE: Mutex<Option<Trace>> = Mutex::new(None);
struct Trace {
    start: Instant,
    events: VecDeque<(u128, &'static str, u32, u64)>,
    history: VecDeque<(u128, String)>,
    incidents: VecDeque<(u128, &'static str, u32, u64)>,
    last_au: Option<u128>,
    last_picture: Option<u128>,
}

pub(crate) fn reset() {
    if let Ok(mut trace) = TRACE.lock() {
        *trace = Some(Trace { start: Instant::now(), events: VecDeque::with_capacity(CAPACITY),
            history: VecDeque::with_capacity(HISTORY_CAPACITY),
            incidents: VecDeque::with_capacity(INCIDENT_CAPACITY), last_au: None, last_picture: None });
    }
}

pub(crate) fn record(stage: &'static str, timestamp: u32, value: u64) {
    if let Ok(mut trace) = TRACE.lock() && let Some(trace) = trace.as_mut() {
        trace.push(trace.start.elapsed().as_micros(), stage, timestamp, value);
    }
}

impl Trace {
    fn incident(&mut self, time: u128, stage: &'static str, timestamp: u32, value: u64) {
        if self.incidents.len() == INCIDENT_CAPACITY { self.incidents.pop_front(); }
        self.incidents.push_back((time, stage, timestamp, value));
    }
    fn push(&mut self, time: u128, stage: &'static str, timestamp: u32, value: u64) {
        if self.events.len() == CAPACITY { self.events.pop_front(); }
        self.events.push_back((time, stage, timestamp, value));
        if matches!(stage, "recovery_begin" | "recovery_end_ms" | "au_abandon" |
            "age_drop" | "output_wait_expired" | "queue_byte_limit" | "queue_frame_limit" |
            "au_queue_pressure_us" | "receiver_ceiling_bps" | "receiver_ceiling_delay_ms" |
            "rtp_gap_released_packets" |
            "decoder_poll_failed" | "keyframe_request" | "manual_refresh" |
            "lag_keyframe_request" | "keyframe_cutover_units")
            || (stage == "au_drop_reason" && value != 5) {
            self.incident(time, stage, timestamp, value);
        }
        let previous = match stage {
            "au_complete" => self.last_au.replace(time),
            "receive_to_gpu_done_us" => self.last_picture.replace(time),
            _ => None,
        };
        if let Some(previous) = previous {
            let gap = time.saturating_sub(previous);
            if gap > 50_000 {
                self.incident(time, if stage == "au_complete" {
                    "au_delivery_gap_us"
                } else { "new_picture_gap_us" }, timestamp, gap.min(u64::MAX as u128) as u64);
            }
        }
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
    let (text, history, incidents) = if let Ok(trace) = TRACE.lock() && let Some(trace) = trace.as_ref() {
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
        let mut incidents = String::from("elapsed_us,stage,submission_rtp_timestamp,value_us_or_reason\n");
        for (time, stage, timestamp, value) in &trace.incidents {
            use std::fmt::Write;
            let _ = writeln!(incidents, "{time},{stage},{timestamp},{value}");
        }
        (text, history, incidents)
    } else { return; };
    let dir = "ux0:data/green-vita-540-test";
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        // Retain one prior session so a refresh does not erase its evidence.
        for name in ["trace.csv", "status.txt", "history.txt", "incidents.csv"] {
            let current = format!("{dir}/pipeline-{name}");
            let previous = format!("{dir}/pipeline-previous-{name}");
            if std::path::Path::new(&current).exists() {
                if std::path::Path::new(&previous).exists() { std::fs::remove_file(&previous)?; }
                std::fs::rename(current, previous)?;
            }
        }
        std::fs::write(format!("{dir}/pipeline-trace.csv"), text)?;
        std::fs::write(format!("{dir}/pipeline-history.txt"), history)?;
        std::fs::write(format!("{dir}/pipeline-incidents.csv"), incidents)?;
        std::fs::write(format!("{dir}/pipeline-status.txt"), status)
    })();
    if let Err(error) = result { eprintln!("Could not save pipeline trace: {error}"); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incidents_outlive_verbose_ring_and_preserve_real_gaps_and_recovery_reason() {
        let mut trace=Trace{start:Instant::now(),events:VecDeque::new(),history:VecDeque::new(),
            incidents:VecDeque::new(),last_au:None,last_picture:None};
        trace.push(0,"au_complete",1,0); trace.push(0,"receive_to_gpu_done_us",1,0);
        trace.push(100_000,"au_complete",2,0);
        trace.push(100_010,"recovery_begin",0,0); trace.push(100_011,"au_drop_reason",0,3);
        trace.push(600_000,"receive_to_gpu_done_us",3,0);
        for time in 600_001..610_001 { trace.push(time,"decoder_poll_return",0,1); }
        assert_eq!(trace.events.len(),CAPACITY);
        assert_eq!(trace.incidents.iter().map(|e|(e.1,e.3)).collect::<Vec<_>>(),vec![
            ("au_delivery_gap_us",100_000),("recovery_begin",0),("au_drop_reason",3),("new_picture_gap_us",600_000)]);
        for time in 0..2000 {trace.push(time,"recovery_begin",0,0);}
        assert_eq!(trace.incidents.len(),INCIDENT_CAPACITY);
    }
}
