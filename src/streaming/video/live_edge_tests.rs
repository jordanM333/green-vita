use super::*;

#[test]
fn auxiliary_sender_report_mismatch_must_not_kill_healthy_video() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    for frame in 0..180u32 {
        let now = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60);
        let ts = 123_456u32.wrapping_add(frame * 1500);
        if frame == 0 {
            edge.establish(ts, now, now);
        }
        edge.observe(ts, frame as u16, now, now);
        if frame == 0 {
            edge.sender_report(ts, 1u64 << 32, now);
        }
        // An optional SR clock observation is inconsistent by one frame.
        // It is not evidence that continuously advancing RTP became stale.
        if frame == 60 {
            edge.sender_report(ts - 1500, 2u64 << 32, now);
        }
        assert!(
            edge.can_present(ts, now),
            "frame {frame}: {}",
            edge.summary()
        );
    }
}

#[test]
fn setup_packets_are_not_a_playback_clock_establishment() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    edge.observe(0, 1, start, start);
    edge.poll(start + Duration::from_secs(1));
    assert_eq!(edge.state(), State::Unmeasured);
    let now = start + Duration::from_secs(1);
    edge.observe(0xd0000000, 2, now, now);
    assert!(edge.can_present(0xd0000000, now));
    assert_eq!(edge.incidents, 0);
}

#[test]
fn fast_forward_catchup_is_an_earlier_edge_not_an_invalid_clock() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    edge.establish(0, start, start);
    // A compressed burst advances media by 600ms in 20ms of dequeue time.
    // That reduces existing unknown path delay; it cannot mean stale media.
    let now = start + Duration::from_millis(20);
    edge.observe(54_000, 1, now, now);
    assert_eq!(edge.state(), State::Live);
    assert!(edge.can_present(54_000, now));
    // This earlier baseline still rejects a subsequent real delay.
    let delayed = now + Duration::from_secs(2);
    assert!(edge.observe(55_500, 2, delayed, delayed));
    assert!(!edge.can_present(55_500, delayed));
    assert!(
        !edge.establish(55_500, delayed, delayed),
        "recovery cannot rebase"
    );
}

#[test]
fn rejected_audio_report_cannot_mute_fresh_samples_or_excuse_delayed_audio() {
    let start = Instant::now();
    let mut clock = MediaClock::new(48_000);
    for packet in 0..150u32 {
        let now = start + Duration::from_millis(u64::from(packet) * 20);
        let ts = packet * 960;
        clock.observe(ts, packet as u16, now);
        if packet == 0 {
            clock.sender_report(ts, 1u64 << 32, now);
        }
        if packet == 50 {
            clock.sender_report(ts - 960, 2u64 << 32, now);
        }
        assert!(clock.valid());
        assert!(now + Duration::from_millis(140) <= clock.deadline(ts).unwrap());
    }
    let now = start + Duration::from_millis(4595);
    clock.observe(144_000, 151, now);
    assert!(clock.deadline(144_000).unwrap() < now);
}

/// Gaps between consecutive request instants.
fn gaps(requests: &[Instant]) -> Vec<Duration> {
    requests.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn frozen_timestamps_and_a_silent_stream_expire_without_repeated_flushes() {
    let start = Instant::now();
    for duplicate_packets in [false, true] {
        let mut edge = LiveEdge::default();
        edge.observe(123, 1, start, start);
        edge.establish(123, start, start);
        let mut transitions = 0;
        let mut requests = Vec::new();
        let end = start + Duration::from_secs(60);
        for tick in 1..=6000u16 {
            let now = start + Duration::from_millis(u64::from(tick) * 10);
            transitions += u32::from(if duplicate_packets {
                edge.observe(123, tick, now, now)
            } else {
                edge.poll(now)
            });
            if edge.request_due(now) {
                requests.push(now);
            }
            if tick > 48 {
                assert!(!edge.can_present(123, now));
            }
        }
        // One quarantine, never a periodic purge. Requests never stop while
        // video waits, but back off 300 ms -> 600 ms -> 1 s and stay there.
        assert_eq!(transitions, 1);
        let gaps = gaps(&requests);
        assert_eq!(
            gaps[..2],
            [Duration::from_millis(300), Duration::from_millis(600)]
        );
        assert!(
            gaps[2..].iter().all(|gap| *gap == REQUEST_CEILING),
            "{gaps:?}"
        );
        assert!(end - *requests.last().unwrap() <= REQUEST_CEILING);
        assert!(requests.len() > 55, "{} requests", requests.len());
        assert_eq!(edge.state(), State::AwaitingKeyframe);
    }
}

#[test]
fn observed_deficit_cannot_become_persistent_stale_playback_even_with_empty_queues() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    let mut requests = Vec::new();
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
        if frame == 0 {
            edge.establish(ts, now, now);
        }
        if edge.observe(ts, frame as u16, now, now) {
            detected_at.get_or_insert(lag_us);
        }
        if edge.request_due(now) {
            requests.push(now);
        }
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
    assert_eq!(edge.incidents, 1, "no periodic purge");
    assert_eq!(
        edge.recovered, 0,
        "no live IDR was supplied; recovery is not proved"
    );
    // Requests continue for the whole plateau at a bounded rate: never faster
    // than the cooldown, at most one per second once backed off.
    let gaps = gaps(&requests);
    assert!(gaps.iter().all(|gap| *gap >= REQUEST_COOLDOWN), "PLI storm");
    let late = &gaps[2..];
    assert!(late.iter().all(|gap| *gap >= REQUEST_CEILING), "PLI storm");
    assert!(
        late.iter()
            .all(|gap| *gap < REQUEST_CEILING + Duration::from_millis(20))
    );
    assert!(
        last - *requests.last().unwrap() <= REQUEST_CEILING,
        "gave up"
    );
    assert_eq!(edge.state(), State::AwaitingKeyframe);
    // Keep the stale plateau running until the next request goes out.
    let mut frame = 108_000u32;
    let sent = loop {
        let now = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60 + 1_595_000);
        edge.observe(frame * 1500, frame as u16, now, now);
        frame += 1;
        if edge.request_due(now) {
            break now;
        }
    };
    // Actual current media returns 100 ms later. Dependent pictures alone
    // cannot restart; the backoff restarts at the cooldown instead of 1 s.
    let current = sent + Duration::from_millis(100);
    let ts = (current.duration_since(start).as_micros() * 90 / 1000) as u32;
    edge.observe(ts, frame as u16, current, current);
    assert!(!edge.admit(ts, false, current));
    assert!(!edge.request_due(sent + Duration::from_millis(299)));
    let rearmed = sent + REQUEST_COOLDOWN;
    assert!(
        edge.request_due(rearmed),
        "current media restarts the backoff"
    );
    assert!(edge.admit(ts, true, rearmed));
    edge.submitted(ts, true, rearmed);
    assert_eq!(edge.recovered, 0, "admission is not presentation");
    assert!(edge.presented(ts, rearmed + Duration::from_millis(80)));
    assert_eq!(edge.state(), State::Live);
    assert_eq!(edge.recovered, 1);
    println!(
        "1595ms model: quarantine at {}us; 30min plateau blocked; {} requests, none closer than {}ms, steady {}ms; current-IDR output restores LIVE",
        detected_at.unwrap(),
        requests.len(),
        gaps.iter().min().unwrap().as_millis(),
        REQUEST_CEILING.as_millis()
    );
}

#[test]
fn legitimate_jitter_duplicates_reorder_bursts_and_idle_do_not_rebase_or_trigger() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    for frame in 0..3600u32 {
        let nominal = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60);
        let now = nominal + Duration::from_millis(if frame % 120 < 6 { 60 } else { 0 });
        if frame == 0 {
            edge.establish(0, now, now);
        }
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
    edge.establish(u32::MAX - 1499, start, start);
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
    edge.establish(0, start, start);
    edge.observe(0, 0, start, start);
    let now = start + Duration::from_secs(2);
    edge.observe(1500, 1, now, now);
    assert!(edge.request_due(now));
    edge.observe(180_000, 2, now, now);
    edge.submitted(180_000, true, now);
    edge.damage();
    assert_eq!(edge.state(), State::AwaitingKeyframe);
    // A failed IDR continues the incident's schedule: neither a fresh burst
    // nor a stop. The next request follows the existing 300 ms gap.
    assert!(!edge.request_due(now + Duration::from_millis(299)));
    assert!(edge.request_due(now + Duration::from_millis(300)));
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
            if frame == 0 {
                edge.establish(ts, now, now);
            }
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
        if frame == 0 {
            edge.establish(0, now, now);
        }
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
