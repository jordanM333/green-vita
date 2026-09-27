#![cfg(test)]
// Compile the production interceptor against exactly the shipped RTC components,
// without linking the Vita-only application or mocking report generation.
extern crate self as rtc;
pub use {interceptor, rtcp, rtp, sansio, shared};
#[path = "../../../src/api/streaming/rtc/clock.rs"]
mod clock;
#[path = "../../../src/streaming/video/freshness.rs"]
pub(crate) mod freshness;
#[path = "../../../src/api/streaming/rtc/reports.rs"]
mod reports;
#[path = "../../../src/streaming/video/timing.rs"]
pub(crate) mod timing;
pub(crate) mod streaming {
    pub(crate) mod video {
        pub(crate) use crate::freshness;
    }
}

#[test]
fn clock_without_sender_report_exposes_relative_timing_not_absolute_age() {
    let mut probe = clock::RtpClockProbe::new(90_000);
    assert!(probe.timing().is_none());
    probe.receive(u32::MAX - 100);
    let first = probe.timing().unwrap();
    assert_eq!(first.timestamp, u32::MAX - 100);
    assert_eq!(first.added_delay_ms, 0);
    assert!(probe.age_ms().is_none());
    assert_eq!(probe.summary(std::time::Instant::now()), "? SR:0 rel+0ms");
    // A duplicate must not move the frame's arrival timestamp.
    probe.receive(first.timestamp);
    assert_eq!(probe.timing().unwrap().received_at, first.received_at);
}

#[test]
fn picture_lifetime_preserves_distinct_receive_submit_and_completion_times() {
    use std::time::{Duration, Instant};
    let start = Instant::now();
    let mut tracker = timing::PictureTracker::default();
    assert!(!tracker.has_pending());
    let submitted = start + Duration::from_millis(10);
    let completed = start + Duration::from_millis(30);
    let pts = tracker.submit(45, start, submitted, 3);
    assert!(tracker.has_pending());
    assert_eq!(tracker.pending_count(), 1);
    assert!(tracker.output(pts + 1, completed).is_none());
    assert_eq!(tracker.pending_count(), 1);
    let frame = tracker.output(pts, completed).unwrap();
    assert_eq!(frame.received_at, start);
    assert_eq!(frame.submitted_at, submitted);
    assert_eq!(frame.decoded_at, completed);
    assert_eq!(frame.epoch, 3);
    assert_eq!(tracker.pending_count(), 0);
    assert!(!tracker.has_pending());
}
