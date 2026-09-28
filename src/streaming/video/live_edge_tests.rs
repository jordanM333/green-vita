use super::*;

#[test]
fn frozen_timestamps_and_a_silent_stream_expire_without_repeated_flushes() {
    let start = Instant::now();
    for duplicate_packets in [false, true] {
        let mut edge = LiveEdge::default();
        edge.observe(123, 1, start, start);
        let mut transitions = 0;
        let mut requests = 0;
        for tick in 1..=6000u16 {
            let now = start + Duration::from_millis(u64::from(tick) * 10);
            transitions += u32::from(if duplicate_packets {
                edge.observe(123, tick, now, now)
            } else {
                edge.poll(now)
            });
            requests += u32::from(edge.request_due(now));
            if tick > 48 {
                assert!(!edge.can_present(123, now));
            }
        }
        assert_eq!(transitions, 1);
        assert_eq!(requests, 3);
        assert_eq!(edge.state(), State::AwaitingKeyframe);
    }
}

#[test]
fn observed_deficit_cannot_become_persistent_stale_playback_even_with_empty_queues() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    let mut requests = 0;
    let mut detected_at = None;
    let mut last = start;
    // Controlled model of the measured 1.595s added delay, then a 30-minute
    // stale plateau. This is timing replay, not a synthetic Xbox/AVCDEC claim.
    for frame in 0..108_000u32 {
        let lag_us = u64::from(frame.saturating_sub(120))
            .saturating_mul(8_000)
            .min(1_595_000);
        let now = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60 + lag_us);
        let ts = frame * 1500;
        if edge.observe(ts, frame as u16, now, now) {
            detected_at.get_or_insert(lag_us);
        }
        requests += u64::from(edge.request_due(now));
        if lag_us > 480_000 {
            assert!(!edge.admit(ts, false, now));
            assert!(
                !edge.admit(ts, true, now),
                "obsolete IDR must not reset the media baseline"
            );
            assert!(!edge.can_present(ts, now));
        }
        last = now;
    }
    assert!(detected_at.unwrap() < 480_000);
    assert_eq!(edge.incidents, 1);
    assert_eq!(
        edge.recovered, 0,
        "no live IDR was supplied; recovery is not proved"
    );
    assert_eq!(requests, 3, "no periodic purge or unbounded PLI storm");
    assert_eq!(edge.state(), State::AwaitingKeyframe);
    // Actual current media returns. Dependent pictures alone cannot restart.
    let ts = (last.duration_since(start).as_micros() * 90 / 1000) as u32;
    edge.observe(ts, 108_000u32 as u16, last, last);
    assert!(!edge.admit(ts, false, last));
    assert!(
        edge.request_due(last),
        "one retry rearmed by an observed fresh edge"
    );
    assert!(edge.admit(ts, true, last));
    edge.submitted(ts, true, last);
    assert_eq!(edge.recovered, 0, "admission is not presentation");
    assert!(edge.presented(ts, last + Duration::from_millis(80)));
    assert_eq!(edge.state(), State::Live);
    assert_eq!(edge.recovered, 1);
    println!(
        "1595ms model: quarantine at {}us; 30min plateau blocked; 3 requests; current-IDR output restores LIVE",
        detected_at.unwrap()
    );
}

#[test]
fn legitimate_jitter_duplicates_reorder_bursts_and_idle_do_not_rebase_or_trigger() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    for frame in 0..3600u32 {
        let nominal = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60);
        let now = nominal + Duration::from_millis(if frame % 120 < 6 { 60 } else { 0 });
        edge.observe(frame * 1500, frame as u16, now, now);
        if frame > 10 {
            edge.observe((frame - 2) * 1500, (frame - 2) as u16, now, now);
            edge.observe(frame * 1500, frame as u16, now, now);
        }
        assert_eq!(edge.state(), State::Live);
    }
    let now = start + Duration::from_secs(70);
    edge.observe(70 * 90_000, 3601, now, now);
    assert!(edge.admit(70 * 90_000, false, now));
    assert!(!edge.can_present(70 * 90_000, now + MEDIA_BUDGET + Duration::from_millis(1)));
    assert_eq!(edge.incidents, 0);
}

#[test]
fn timestamp_and_sequence_wrap_are_not_resets_but_discontinuity_is_unknown() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    edge.observe(u32::MAX - 1499, u16::MAX, start, start);
    let now = start + Duration::from_micros(16_667);
    edge.observe(0, 0, now, now);
    assert!(edge.admit(0, false, now));
    assert!(edge.added_delay_ms(0, now).unwrap() < 1);
    edge.observe(0u32.wrapping_sub(90_000 * 3), 1, now, now);
    assert_eq!(edge.state(), State::ClockUncertain);
    assert!(!edge.admit(0, true, now));
    assert!(!edge.request_due(now));
    // Only an explicit new source/session lifecycle may establish a new origin.
    edge = LiveEdge::default();
    edge.observe(17, 2, now, now);
    assert!(edge.admit(17, true, now));
}

#[test]
fn failed_or_expired_keyframe_does_not_report_recovery_or_restart_request_budget() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    edge.observe(0, 0, start, start);
    let now = start + Duration::from_secs(2);
    edge.observe(1500, 1, now, now);
    assert!(edge.request_due(now));
    edge.observe(180_000, 2, now, now);
    edge.submitted(180_000, true, now);
    edge.damage();
    assert_eq!(edge.state(), State::AwaitingKeyframe);
    assert!(!edge.presented(180_000, now));
    edge.submitted(180_000, true, now);
    assert!(!edge.presented(180_000, now + MEDIA_BUDGET + Duration::from_millis(1)));
    assert_eq!(edge.recovered, 0);
}

#[test]
fn six_hour_clock_skew_is_calibrated_without_forgiving_media_lag() {
    for ppm in [-500i64, 0, 500] {
        let start = Instant::now();
        let mut edge = LiveEdge::default();
        for frame in 0..=6 * 3600 * 60u64 {
            let media_ns = frame * 1_000_000_000 / 60;
            let wall_ns = (i128::from(media_ns) * 1_000_000 / i128::from(1_000_000 + ppm)) as u64;
            let now = start + Duration::from_nanos(wall_ns);
            let ts = (frame * 1500) as u32;
            edge.observe(ts, frame as u16, now, now);
            if frame % 60 == 0 {
                let ntp = (u128::from(media_ns) * (1u128 << 32) / 1_000_000_000) as u64;
                edge.sender_report(ts, ntp, now);
            }
            assert!(
                edge.admit(ts, false, now),
                "{ppm}ppm frame {frame}: {}",
                edge.summary()
            );
        }
        assert_eq!(edge.incidents, 0);
    }
}

#[test]
fn delayed_sender_reports_cannot_calibrate_away_a_stale_plateau() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    for frame in 0..7200u32 {
        let media = Duration::from_micros(u64::from(frame) * 1_000_000 / 60);
        let lag = if frame < 120 {
            Duration::ZERO
        } else {
            Duration::from_millis(1595)
        };
        let now = start + media + lag;
        edge.observe(frame * 1500, frame as u16, now, now);
        if frame % 60 == 0 {
            edge.sender_report(frame * 1500, (u64::from(frame) / 60) << 32, now);
        }
        if frame > 120 {
            assert!(!edge.admit(frame * 1500, true, now));
        }
    }
    assert_eq!(edge.recovered, 0);
}
