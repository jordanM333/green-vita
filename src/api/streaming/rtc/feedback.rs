//! Negotiated REMB feedback with delay-based congestion response and rate measurements.
use std::time::{Duration, Instant};
#[path = "congestion.rs"]
mod congestion;

#[path = "bandwidth.rs"]
mod bandwidth;
pub(crate) use bandwidth::VIDEO_CEILING_BPS;
const FEEDBACK_INTERVAL: Duration = Duration::from_millis(500);

/// Use feedback only for an accepted video payload in the remote answer. The
/// Xbox offer already advertises goog-remb; audio and rejected m-lines do not opt in.
pub(crate) fn remb_payloads(sdp: &str) -> Vec<u8> {
    feedback_payloads(sdp, "goog-remb")
}

pub(crate) fn feedback_payloads(sdp: &str, feedback: &str) -> Vec<u8> {
    let mut video = false;
    let mut accepted = Vec::new();
    let mut result = Vec::new();
    for line in sdp.lines().map(str::trim) {
        if line.starts_with("m=") {
            let fields: Vec<_> = line.split_whitespace().collect();
            video = fields.first() == Some(&"m=video")
                && fields.get(1).and_then(|port| port.split('/').next())
                    .and_then(|port| port.parse::<u16>().ok()).is_some_and(|port| port != 0);
            accepted = if video {
                fields.iter().skip(3).filter_map(|s| s.parse::<u8>().ok()).collect()
            } else { Vec::new() };
        } else if video && let Some(value) = line.strip_prefix("a=rtcp-fb:") {
            let fields: Vec<_> = value.split_whitespace().collect();
            if fields.get(1) == Some(&feedback) && fields.len() == 2 {
                if fields.first() == Some(&"*") {
                    result.extend(accepted.iter().copied());
                } else if let Some(pt) = fields.first().and_then(|s| s.parse::<u8>().ok()) {
                    if accepted.contains(&pt) { result.push(pt); }
                }
            }
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

pub(crate) struct VideoCeiling {
    budget: congestion::ReceiveBudget,
    supported_payloads: Vec<u8>,
    active: bool,
    last_attempt: Option<Instant>,
    queued: u64,
    failed: u64,
    over_windows: u64,
}

impl Default for VideoCeiling {
    fn default() -> Self {
        Self { budget: congestion::ReceiveBudget::new(VIDEO_CEILING_BPS),
            supported_payloads: Vec::new(), active: false, last_attempt: None,
            queued: 0, failed: 0, over_windows: 0 }
    }
}

impl VideoCeiling {
    pub(crate) fn receive(&mut self, bytes: usize, delay_ms: u64, now: Instant) {
        self.budget.receive(bytes, delay_ms, now);
    }
    pub(crate) fn target_bps(&self) -> u32 { self.budget.target() }

    pub(crate) fn answer(&mut self, sdp: &str) {
        *self = Self { supported_payloads: remb_payloads(sdp), ..Default::default() };
    }
    pub(crate) fn observe_payload(&mut self, payload: u8) {
        self.active = self.supported_payloads.contains(&payload);
    }
    pub(crate) fn due(&self, now: Instant) -> bool {
        self.active && self.last_attempt.is_none_or(|at| now.duration_since(at) >= FEEDBACK_INTERVAL)
    }
    pub(crate) fn attempted(&mut self, now: Instant, success: bool) {
        self.last_attempt = Some(now);
        if success { self.queued += 1; } else { self.failed += 1; }
    }
    pub(crate) fn summary(&mut self, measured_kbps: u64) -> String {
        // This is an observation, not a server acknowledgement or hard wire cap.
        // Allow 25% for short-window/keyframe bursts before counting an overshoot.
        if self.queued >= 4 && measured_kbps > u64::from(self.budget.target()) / 800 {
            self.over_windows += 1;
        }
        let state = if self.active { "adaptive" }
            else if self.supported_payloads.is_empty() { "unsupported" }
            else { "waiting-video" };
        format!("REMB:{state} target:{}k delay:{}ms cuts:{} queued:{} fail:{} over-windows:{}", self.budget.target() / 1000, self.budget.delay_ms, self.budget.reductions, self.queued, self.failed, self.over_windows)
    }
}

pub(crate) struct ReceiveRate {
    started: Instant,
    bytes: u64,
    bucket_start: Instant,
    bucket_bytes: u64,
    peak_kbps: u64,
    last_packet: Option<Instant>,
    max_gap_ms: u128,
    pub(crate) latest_kbps: u64,
}

impl ReceiveRate {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self { started: now, bytes: 0, bucket_start: now, bucket_bytes: 0,
            peak_kbps: 0, last_packet: None, max_gap_ms: 0, latest_kbps: 0 }
    }

    fn close_bucket(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.bucket_start).as_millis();
        if elapsed >= 100 {
            self.peak_kbps = self.peak_kbps.max(self.bucket_bytes * 8 / elapsed as u64);
            self.bucket_start = now;
            self.bucket_bytes = 0;
        }
    }

    pub(crate) fn receive(&mut self, bytes: usize, now: Instant) {
        self.close_bucket(now);
        if let Some(last) = self.last_packet {
            self.max_gap_ms = self.max_gap_ms.max(now.duration_since(last).as_millis());
        }
        self.last_packet = Some(now);
        self.bytes += bytes as u64;
        self.bucket_bytes += bytes as u64;
    }

    pub(crate) fn summary(&mut self, now: Instant) -> String {
        self.close_bucket(now);
        let elapsed = now.duration_since(self.started).as_millis().max(1);
        let kbps = self.bytes * 8 / elapsed as u64;
        self.latest_kbps = kbps;
        let idle = self.last_packet.map(|last| now.duration_since(last).as_millis()).unwrap_or(0);
        let text = format!("{kbps}/{}k gap:{}/{}ms", self.peak_kbps, idle, self.max_gap_ms.max(idle));
        self.started = now;
        self.bytes = 0;
        self.peak_kbps = 0;
        self.max_gap_ms = 0;
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_use_elapsed_time_and_report_stalls() {
        let mut rate = ReceiveRate::new();
        let start = rate.started;
        rate.receive(25_000, start);
        rate.receive(25_000, start + Duration::from_millis(100));
        assert_eq!(rate.summary(start + Duration::from_millis(200)), "2000/2000k gap:100/100ms");
        assert_eq!(rate.summary(start + Duration::from_millis(1200)), "0/0k gap:1100/1100ms");
    }

    #[test]
    fn remb_requires_accepted_video_payload_and_does_not_leak_across_media() {
        assert_eq!(remb_payloads("m=audio 9 UDP/TLS/RTP/SAVPF 111\na=rtcp-fb:* goog-remb\nm=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 goog-remb\na=rtcp-fb:103 goog-remb"), vec![102]);
        for port in ["0", "0/2", "bad"] {
            assert!(remb_payloads(&format!("m=video {port} UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:* goog-remb")).is_empty());
        }
        assert!(remb_payloads("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 nack pli").is_empty());
        assert_eq!(remb_payloads("m=video 9 UDP/TLS/RTP/SAVPF 102 103\na=rtcp-fb:* goog-remb"), vec![102, 103]);
    }

    #[test]
    fn generic_nack_requires_its_own_negotiated_video_feedback() {
        let sdp = "m=audio 9 UDP/TLS/RTP/SAVPF 111\na=rtcp-fb:* nack\nm=video 9 UDP/TLS/RTP/SAVPF 102 103\na=rtcp-fb:102 nack pli\na=rtcp-fb:103 nack";
        assert_eq!(feedback_payloads(sdp, "nack"), vec![103]);
        assert!(feedback_payloads("m=video 0 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:* nack", "nack").is_empty());
    }

    #[test]
    fn ceiling_requires_matching_payload_and_throttles_failed_attempts_too() {
        let mut cap = VideoCeiling::default();
        let now = Instant::now();
        cap.answer("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 goog-remb");
        assert!(!cap.due(now));
        cap.observe_payload(103);
        assert!(!cap.due(now));
        cap.observe_payload(102);
        assert!(cap.due(now));
        cap.attempted(now, false);
        assert!(!cap.due(now + Duration::from_millis(499)));
        assert!(cap.due(now + FEEDBACK_INTERVAL));
        for i in 1..=4 { cap.attempted(now + FEEDBACK_INTERVAL * i, true); }
        assert!(cap.summary(7291).ends_with("over-windows:1"));
        assert!(cap.summary(2000).ends_with("over-windows:1"));
        cap.answer("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 nack pli");
        cap.observe_payload(102);
        assert!(!cap.due(now));
    }
}
