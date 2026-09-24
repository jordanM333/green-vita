//! Negotiated REMB ceiling; legacy delay adaptation runs only without active TWCC.
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
    arrival: ArrivalFeedback,
    delay_ms: u64,
}

/// A negotiated extension alone is insufficient: use sender-side adaptation
/// only while authenticated video extensions AND successful TWCC sends advance.
#[derive(Default)]
struct ArrivalFeedback {
    packets: u64,
    reports: u64,
    packet_at: Option<Instant>,
    report_at: Option<Instant>,
    active: bool,
}

impl ArrivalFeedback {
    fn update(&mut self, packets: u64, reports: u64, now: Instant) {
        if packets < self.packets || reports < self.reports { *self = Self::default(); }
        if packets > self.packets { self.packet_at = Some(now); }
        if reports > self.reports { self.report_at = Some(now); }
        self.packets = packets;
        self.reports = reports;
        let recent = |at: Option<Instant>| at.is_some_and(|at|
            now.saturating_duration_since(at) < Duration::from_secs(2));
        self.active = recent(self.packet_at) && recent(self.report_at);
    }
}

impl Default for VideoCeiling {
    fn default() -> Self {
        Self { budget: congestion::ReceiveBudget::new(VIDEO_CEILING_BPS),
            supported_payloads: Vec::new(), active: false, last_attempt: None,
            queued: 0, failed: 0, over_windows: 0,
            arrival: Default::default(), delay_ms: 0 }
    }
}

impl VideoCeiling {
    pub(crate) fn receive(&mut self, bytes: usize, delay_ms: u64, now: Instant) {
        self.delay_ms = delay_ms;
        if !self.arrival.active { self.budget.receive(bytes, delay_ms, now); }
    }
    pub(crate) fn target_bps(&self) -> u32 {
        if self.arrival.active { VIDEO_CEILING_BPS } else { self.budget.target() }
    }

    pub(crate) fn update_arrival_feedback(&mut self, video_packets: u64, sent: u64, now: Instant) {
        let previous = self.arrival.active;
        self.arrival.update(video_packets, sent, now);
        if previous != self.arrival.active {
            // Do not revive the poisoned pre-TWCC estimate on fallback, or let
            // legacy REMB reductions overrule the working sender controller.
            self.budget = congestion::ReceiveBudget::new(VIDEO_CEILING_BPS);
            self.last_attempt = None;
        }
    }

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
        if self.queued >= 4 && measured_kbps > u64::from(self.target_bps()) / 800 {
            self.over_windows += 1;
        }
        let state = if self.active && self.arrival.active { "twcc-cap" }
            else if self.active { "adaptive" }
            else if self.supported_payloads.is_empty() { "unsupported" }
            else { "waiting-video" };
        format!("REMB:{state} target:{}k delay:{}ms cuts:{} queued:{} fail:{} over-windows:{}", self.target_bps() / 1000, self.delay_ms, self.budget.reductions, self.queued, self.failed, self.over_windows)
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
    fn working_twcc_keeps_one_controller_through_ten_minutes_of_delay_and_pause() {
        let start = Instant::now();
        let mut cap = VideoCeiling::default();
        cap.answer("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 goog-remb");
        cap.observe_payload(102);
        for tick in 1..=6000 {
            let now = start + Duration::from_millis(tick * 100);
            cap.update_arrival_feedback(tick * 10, tick, now);
            // Busy video, a low-complexity paused scene, then busy again.
            // A persistent offset must not introduce a second estimator.
            let bytes = if (2000..4000).contains(&tick) { 500 } else { 50_000 };
            cap.receive(bytes, if tick < 100 { 5 } else { 1991 }, now);
            assert_eq!(cap.target_bps(), VIDEO_CEILING_BPS);
        }
        assert_eq!(cap.budget.reductions, 0);
        assert!(cap.summary(980).starts_with("REMB:twcc-cap target:2000k"));
        assert!(cap.summary(980).contains("delay:1991ms"), "do not hide latency");
    }

    #[test]
    fn confirmed_arrival_feedback_releases_a_previous_legacy_clamp_immediately() {
        let start = Instant::now();
        let mut cap = VideoCeiling::default();
        cap.answer("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 goog-remb");
        cap.observe_payload(102);
        for tick in 0..=100 {
            cap.receive(25_000, 200, start + Duration::from_millis(tick * 100));
        }
        assert_eq!(cap.target_bps(), 500_000, "reproduce old controller clamp");
        let now = start + Duration::from_secs(11);
        cap.attempted(now, true);
        cap.update_arrival_feedback(10, 1, now);
        assert_eq!(cap.target_bps(), VIDEO_CEILING_BPS);
        assert!(cap.due(now), "send the corrected ceiling without another half-second wait");
    }

    #[test]
    fn negotiation_or_one_way_feedback_alone_cannot_disable_fallback() {
        let start = Instant::now();
        let mut state = ArrivalFeedback::default();
        state.update(0, 20, start); // audio reports alone
        assert!(!state.active);
        state.update(10, 20, start + Duration::from_secs(3)); // TX stalled
        assert!(!state.active);
        state.update(11, 21, start + Duration::from_secs(3));
        assert!(state.active);
        state.update(11, 40, start + Duration::from_secs(5)); // no recent video
        assert!(!state.active);
        state.update(12, 41, start + Duration::from_secs(6));
        assert!(state.active);
        state.update(0, 0, start + Duration::from_secs(7)); // new session
        assert!(!state.active);
    }

    #[test]
    fn stalled_arrival_feedback_falls_back_and_new_answer_forgets_old_mode() {
        let start = Instant::now();
        let mut cap = VideoCeiling::default();
        cap.update_arrival_feedback(10, 1, start);
        assert!(cap.arrival.active);
        cap.update_arrival_feedback(10, 1, start + Duration::from_secs(2));
        assert!(!cap.arrival.active);
        for tick in 20..=100 {
            cap.receive(25_000, 300, start + Duration::from_millis(tick * 100));
        }
        assert!(cap.target_bps() < VIDEO_CEILING_BPS);
        cap.update_arrival_feedback(20, 2, start + Duration::from_secs(11));
        assert!(cap.arrival.active);
        cap.answer("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 nack");
        cap.observe_payload(102);
        assert!(!cap.arrival.active);
        assert!(!cap.due(start + Duration::from_secs(12)));
    }

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
