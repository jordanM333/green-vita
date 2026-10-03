//! Sender RTP/RTCP clock probe. This only measures timestamps; it never delays media.
use rtcp::sender_report::SenderReport;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const NTP_UNIX_SECONDS: i128 = 2_208_988_800;
const MAX_REPORT_DISTANCE_SECONDS: i64 = 120;

/// Independent, allocation-free observation of the socket -> RTC handoff.
/// Never subtract two independently minimized relative clocks to infer residence:
/// measure the same packet's two Instants instead.
pub(super) struct IngressProbe {
    clock: RtpClockProbe,
    packets: u64,
    residence_us: u128,
    residence_max_us: u128,
    previous_ticks: u64,
    previous_at: Option<Instant>,
}

impl IngressProbe {
    pub(super) fn new(rate: i64) -> Self {
        Self {
            clock: RtpClockProbe::new(rate),
            packets: 0,
            residence_us: 0,
            residence_max_us: 0,
            previous_ticks: 0,
            previous_at: None,
        }
    }

    pub(super) fn rate(&self) -> u64 {
        self.clock.clock_rate as u64
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new(self.clock.clock_rate);
    }

    pub(super) fn receive(&mut self, timestamp: u32, dequeued: Instant, delivered: Instant) {
        self.clock.receive_at(timestamp, dequeued);
        let us = delivered.saturating_duration_since(dequeued).as_micros();
        self.packets += 1;
        self.residence_us += us;
        self.residence_max_us = self.residence_max_us.max(us);
    }

    /// Added arrival delay of the latest packet, if it arrived within `recent`
    /// of `now`. HA07 compares audio with video to tell a shared-path queue
    /// (both late) from the Xbox's own video queue (only video late).
    pub(super) fn recent_delay_ms(&self, now: Instant, recent: Duration) -> Option<u64> {
        let timing = self.clock.timing()?;
        (now.saturating_duration_since(timing.received_at) <= recent)
            .then_some(timing.added_delay_ms)
    }

    pub(super) fn take_summary(&mut self, now: Instant) -> String {
        let media_us =
            self.clock.elapsed_ticks.saturating_sub(self.previous_ticks) * 1_000_000 / self.rate();
        let wall_us = self
            .previous_at
            .map(|at| now.saturating_duration_since(at).as_micros());
        let progress = wall_us
            .map(|wall| format!("{media_us}/{wall}us"))
            .unwrap_or_else(|| "?".into());
        let avg = self
            .residence_us
            .checked_div(u128::from(self.packets))
            .unwrap_or(0);
        let relative = self
            .clock
            .timing()
            .map(|timing| timing.added_delay_ms.to_string())
            .unwrap_or_else(|| "?".into());
        let summary = format!(
            "dequeue rel+{relative}ms RTC avg/max:{avg}/{}us pk:{} media/wall:{progress}",
            self.residence_max_us, self.packets,
        );
        self.previous_ticks = self.clock.elapsed_ticks;
        self.previous_at = Some(now);
        self.packets = 0;
        self.residence_us = 0;
        self.residence_max_us = 0;
        summary
    }
}

pub(super) struct RtpClockProbe {
    clock_rate: i64,
    report: Option<(u32, i128)>,
    last_report_at: Option<Instant>,
    last_timestamp: Option<u32>,
    latest_age_ms: Option<i64>,
    baseline_age_ms: Option<i64>,
    report_count: u64,
    arrival_baseline: Option<Instant>,
    elapsed_ticks: u64,
    minimum_offset_us: i128,
    relative_delay_ms: u64,
    last_frame_at: Option<Instant>,
}

impl RtpClockProbe {
    pub(super) fn new(clock_rate: i64) -> Self {
        Self {
            clock_rate,
            report: None,
            last_report_at: None,
            last_timestamp: None,
            latest_age_ms: None,
            baseline_age_ms: None,
            report_count: 0,
            arrival_baseline: None,
            elapsed_ticks: 0,
            minimum_offset_us: 0,
            relative_delay_ms: 0,
            last_frame_at: None,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new(self.clock_rate);
    }

    pub(super) fn sender_report(&mut self, sr: &SenderReport) {
        self.last_report_at = Some(Instant::now());
        self.report_count += 1;
        let seconds = i128::from(sr.ntp_time >> 32);
        if seconds < NTP_UNIX_SECONDS {
            return;
        }
        let fraction = i128::from(sr.ntp_time as u32);
        let unix_ms = (seconds - NTP_UNIX_SECONDS) * 1_000 + ((fraction * 1_000) >> 32);
        self.report = Some((sr.rtp_time, unix_ms));
    }

    pub(super) fn receive(&mut self, timestamp: u32) {
        self.receive_at(timestamp, Instant::now());
    }

    fn receive_at(&mut self, timestamp: u32, now: Instant) {
        if let Some(previous) = self.last_timestamp {
            let forward = timestamp.wrapping_sub(previous);
            // Ignore duplicate timestamps and late/reordered AUs, including across wrap.
            if forward == 0 || forward >= (1 << 31) {
                return;
            }
            if i64::from(forward) > self.clock_rate * MAX_REPORT_DISTANCE_SECONDS {
                self.arrival_baseline = None;
                self.elapsed_ticks = 0;
                self.minimum_offset_us = 0;
            } else {
                self.elapsed_ticks += u64::from(forward);
            }
        }
        self.last_timestamp = Some(timestamp);
        self.last_frame_at = Some(now);
        let baseline = *self.arrival_baseline.get_or_insert(now);
        let media_us = i128::from(self.elapsed_ticks) * 1_000_000 / i128::from(self.clock_rate);
        let offset_us = now.saturating_duration_since(baseline).as_micros() as i128 - media_us;
        self.minimum_offset_us = self.minimum_offset_us.min(offset_us);
        self.relative_delay_ms = ((offset_us - self.minimum_offset_us) / 1_000) as u64;
        let Some((report_rtp, report_unix_ms)) = self.report else {
            return;
        };
        let ticks = i64::from(timestamp.wrapping_sub(report_rtp) as i32);
        if ticks.abs() > self.clock_rate * MAX_REPORT_DISTANCE_SECONDS {
            return;
        }
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return;
        };
        let sender_time_ms = report_unix_ms + i128::from(ticks * 1_000 / self.clock_rate);
        let age_ms = now.as_millis() as i128 - sender_time_ms;
        let Ok(age_ms) = i64::try_from(age_ms) else {
            return;
        };
        self.latest_age_ms = Some(age_ms);
        self.baseline_age_ms.get_or_insert(age_ms);
    }

    pub(super) fn summary(&self, now: Instant) -> String {
        if self.report_count == 0 {
            // This works without RTCP SR or synchronized clocks. It measures added delay
            // since the fastest observed arrival, NOT capture age or fixed network delay.
            return format!("? SR:0 rel+{}ms", self.relative_delay_ms);
        }
        let report_age = self
            .last_report_at
            .map(|time| now.saturating_duration_since(time).as_secs())
            .unwrap_or_default();
        let Some(age_ms) = self.latest_age_ms else {
            return format!(
                "? SR:{}/{}s rel+{}ms",
                self.report_count, report_age, self.relative_delay_ms
            );
        };
        let drift = age_ms - self.baseline_age_ms.unwrap_or(age_ms);
        // Absolute age assumes Xbox and Vita have synchronized clocks. Drift does not.
        format!(
            "{age_ms}ms d{drift:+} rel+{}ms SR:{}/{}s",
            self.relative_delay_ms, self.report_count, report_age
        )
    }

    pub(super) fn timing(&self) -> Option<crate::streaming::video::freshness::VideoTiming> {
        Some(crate::streaming::video::freshness::VideoTiming {
            timestamp: self.last_timestamp?,
            received_at: self.last_frame_at?,
            added_delay_ms: self.relative_delay_ms,
        })
    }

    pub(super) fn age_ms(&self) -> Option<i64> {
        self.latest_age_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_delay_is_reported_only_while_packets_keep_arriving() {
        use std::time::Duration;
        let start = Instant::now();
        let mut audio = IngressProbe::new(48_000);
        assert_eq!(audio.recent_delay_ms(start, Duration::from_secs(1)), None);
        audio.receive(0, start, start);
        // 20 ms of media arriving 60 ms later: 40 ms of added delay.
        audio.receive(960, start + Duration::from_millis(60), start);
        let last = start + Duration::from_millis(60);
        assert_eq!(
            audio.recent_delay_ms(last, Duration::from_secs(1)),
            Some(40)
        );
        assert_eq!(
            audio.recent_delay_ms(last + Duration::from_millis(1_001), Duration::from_secs(1)),
            None,
            "stale audio is not evidence about the path"
        );
    }

    #[test]
    fn ingress_separates_dequeue_staleness_from_two_second_rtc_residence() {
        use std::time::Duration;
        let start = Instant::now();
        let mut upstream = IngressProbe::new(90_000);
        let mut inside_rtc = IngressProbe::new(90_000);
        for probe in [&mut upstream, &mut inside_rtc] {
            probe.receive(0, start, start);
            probe.take_summary(start);
        }
        let delivered = start + Duration::from_secs(3);
        // The old app-only clock is identical in these two distinct mechanisms.
        upstream.receive(90_000, delivered, delivered);
        inside_rtc.receive(90_000, start + Duration::from_secs(1), delivered);
        assert_eq!(upstream.clock.relative_delay_ms, 2000);
        assert_eq!(upstream.residence_max_us, 0);
        assert_eq!(inside_rtc.clock.relative_delay_ms, 0);
        assert_eq!(inside_rtc.residence_max_us, 2_000_000);
        assert!(
            upstream
                .take_summary(delivered)
                .contains("media/wall:1000000/3000000us")
        );
        assert!(
            inside_rtc
                .take_summary(delivered)
                .contains("RTC avg/max:2000000/2000000us")
        );
        inside_rtc.reset();
        assert!(inside_rtc.clock.timing().is_none());
        assert!(inside_rtc.take_summary(delivered).contains("media/wall:?"));
    }

    #[test]
    fn thirty_minutes_of_bursts_wrap_reorder_and_recovery_do_not_accumulate_clock_error() {
        use std::time::Duration;
        let start = Instant::now();
        let first = u32::MAX - 90_000;
        let mut video = IngressProbe::new(90_000);
        let mut audio = IngressProbe::new(48_000);
        for frame in 0..108_000u64 {
            let ts = first.wrapping_add((frame * 1500) as u32);
            let media_us = frame * 1_000_000 / 60;
            let phase = frame % 3600;
            let extra_us = if phase < 3420 {
                0
            } else if phase < 3480 {
                (phase - 3420) * 1_000_000 / 30
            } else {
                (3600 - phase) * 1_000_000 / 60
            };
            // Deliver an old packet after each current one without moving the clock.
            let dequeued = start + Duration::from_micros(media_us + extra_us);
            video.receive(ts, dequeued, dequeued + Duration::from_micros(300));
            let last = video.clock.last_frame_at;
            video.receive(
                ts.wrapping_sub(1500),
                dequeued,
                dequeued + Duration::from_micros(700),
            );
            assert_eq!(video.clock.last_frame_at, last);
            assert!(video.clock.relative_delay_ms.abs_diff(extra_us / 1000) <= 1);
            let audio_at = start + Duration::from_micros(media_us);
            audio.receive(
                (frame * 800) as u32,
                audio_at,
                audio_at + Duration::from_micros(300),
            );
            assert_eq!(audio.clock.relative_delay_ms, 0);
            if frame % 60 == 59 {
                video.take_summary(dequeued);
                audio.take_summary(audio_at);
            }
        }
    }

    #[test]
    fn sender_report_ntp_and_rtp_wrap_are_mapped_to_unix_time() {
        let mut clock = RtpClockProbe::new(90_000);
        let second = NTP_UNIX_SECONDS as u64 + 1_700_000_000;
        clock.sender_report(&SenderReport {
            ntp_time: second << 32,
            rtp_time: u32::MAX - 44_999,
            ..Default::default()
        });
        assert_eq!(clock.report, Some((u32::MAX - 44_999, 1_700_000_000_000)));
        assert_eq!(
            i64::from(45_000_u32.wrapping_sub(u32::MAX - 44_999) as i32),
            90_000
        );
    }

    #[test]
    fn zero_ntp_is_ignored() {
        let mut clock = RtpClockProbe::new(48_000);
        clock.sender_report(&SenderReport::default());
        assert!(clock.report.is_none());
    }

    #[test]
    fn relative_delay_detects_backlog_without_sender_reports() {
        let mut clock = RtpClockProbe::new(90_000);
        let start = Instant::now();
        clock.receive_at(0, start);
        clock.receive_at(90_000, start + std::time::Duration::from_millis(1_500));
        assert_eq!(clock.relative_delay_ms, 500);
        clock.receive_at(180_000, start + std::time::Duration::from_secs(2));
        assert_eq!(clock.relative_delay_ms, 0);
    }

    #[test]
    fn relative_delay_handles_wrap_and_ignores_late_packets() {
        let mut clock = RtpClockProbe::new(90_000);
        let start = Instant::now();
        let first = u32::MAX - 44_999;
        clock.receive_at(first, start);
        clock.receive_at(45_000, start + std::time::Duration::from_secs(1));
        assert_eq!(clock.relative_delay_ms, 0);
        clock.receive_at(first, start + std::time::Duration::from_secs(2));
        assert_eq!(clock.last_timestamp, Some(45_000));
        assert_eq!(clock.elapsed_ticks, 90_000);
    }
}
