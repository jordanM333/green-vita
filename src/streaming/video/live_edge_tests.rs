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
    // This earlier baseline still detects a subsequent delay beyond the ceiling.
    let delayed = now + Duration::from_secs(3);
    assert!(edge.observe(55_500, 2, delayed, delayed));
    assert!(!edge.can_present(55_500, delayed));
    assert!(
        !edge.establish(55_500, delayed, delayed),
        "recovery cannot rebase"
    );
}

#[test]
fn a_rejected_report_cannot_invalidate_the_clock_or_excuse_delay() {
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
        assert_eq!(clock.delay(ts, now), Some(Duration::ZERO));
    }
    let now = start + Duration::from_millis(4595);
    clock.observe(144_000, 151, now);
    assert_eq!(clock.delay(144_000, now), Some(Duration::from_millis(1595)));
}

/// Gaps between consecutive request instants.
fn gaps(requests: &[Instant]) -> Vec<Duration> {
    requests.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn a_silent_stream_keeps_asking_for_a_keyframe_without_an_incident() {
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
            // The last picture is too old to present only beyond the ceiling.
            assert_eq!(edge.can_present(123, now), now - start <= LAG_CEILING);
        }
        // Silence is not an incident: if video resumes, its reference chain is
        // intact. Requests start after SILENCE and never stop, backing off
        // 300 ms -> 600 ms -> 1 s.
        assert_eq!(transitions, 0);
        assert_eq!(edge.state(), State::Live);
        assert_eq!(requests[0] - start, SILENCE);
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
        // Video returns: no keyframe is needed, and the schedule resets.
        let back = end + Duration::from_millis(10);
        let ts = 123u32.wrapping_add((back - start).as_micros() as u32 * 9 / 100);
        assert!(!edge.observe(ts, 9_000, back, back));
        assert!(edge.admit(ts, false, back));
        assert!(!edge.request_due(back));
    }
}

#[test]
fn a_sender_backlog_plays_late_and_catches_up_without_a_keyframe() {
    let start = Instant::now();
    let mut edge = LiveEdge::default();
    let mut last = start;
    // Controlled model of the measured 1.595 s added delay (DIAG03-8), held
    // for 30 minutes, then drained at twice real time. Timing replay only.
    let mut frame = 0u32;
    while frame < 108_000 {
        let lag_us = u64::from(frame.saturating_sub(120))
            .saturating_mul(8_000)
            .min(1_595_000);
        let now = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60 + lag_us);
        let ts = frame * 1500;
        if frame == 0 {
            edge.establish(ts, now, now);
        }
        assert!(!edge.observe(ts, frame as u16, now, now), "frame {frame}");
        assert!(edge.admit(ts, false, now));
        assert!(edge.can_present(ts, now));
        assert!(!edge.request_due(now));
        last = now;
        frame += 1;
    }
    assert_eq!(edge.added_delay_ms(1500 * (frame - 1), last), Some(1595));
    // The backlog drains: frames arrive twice as fast until they are current.
    loop {
        let captured = start + Duration::from_micros(u64::from(frame) * 1_000_000 / 60);
        let now = (last + Duration::from_micros(8_333)).max(captured);
        let ts = frame * 1500;
        assert!(!edge.observe(ts, frame as u16, now, now));
        assert!(edge.can_present(ts, now));
        last = now;
        frame += 1;
        if now == captured {
            break;
        }
    }
    assert_eq!(edge.added_delay_ms(1500 * (frame - 1), last), Some(0));
    assert_eq!(edge.state(), State::Live);
    assert_eq!((edge.incidents, edge.requested), (0, 0));
    // Beyond the ceiling is an incident, and only a current keyframe recovers.
    let late = last + LAG_CEILING + Duration::from_millis(100);
    let ts = frame * 1500;
    assert!(edge.observe(ts, frame as u16, late, late));
    assert!(!edge.can_present(ts, late));
    assert!(edge.request_due(late));
    let current = late + Duration::from_millis(100);
    let ts = (current.duration_since(start).as_micros() * 90 / 1000) as u32;
    edge.observe(ts, frame.wrapping_add(1) as u16, current, current);
    assert!(!edge.admit(ts, false, current));
    assert!(edge.admit(ts, true, current));
    edge.submitted(ts, true, current);
    assert_eq!(edge.recovered, 0, "admission is not presentation");
    assert!(edge.presented(ts, current + Duration::from_millis(80)));
    assert_eq!(
        (edge.state(), edge.incidents, edge.recovered),
        (State::Live, 1, 1)
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
    assert!(!edge.can_present(70 * 90_000, now + LAG_CEILING + Duration::from_millis(1)));
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
    // A broken reference chain is an incident; requests follow the backoff.
    let now = start + Duration::from_secs(2);
    edge.observe(178_500, 1, now, now);
    edge.damage();
    assert_eq!(edge.state(), State::AwaitingKeyframe);
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
    assert!(!edge.presented(180_000, now + LAG_CEILING + Duration::from_millis(1)));
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
        // The lag is measured and never calibrated away (HA06 plays it).
        if frame > 120 {
            assert_eq!(edge.added_delay_ms(frame * 1500, now), Some(1595));
        }
    }
    assert_eq!((edge.incidents, edge.recovered), (0, 0));
}

#[test]
fn a_keyframe_requested_into_a_backlog_is_not_requested_again_until_it_can_arrive() {
    let start = Instant::now();
    let after = |ms: u64| start + Duration::from_millis(ms);
    let ts = |ms: u64| (ms * 90) as u32;
    let mut edge = LiveEdge::default();
    edge.establish(0, start, start);
    edge.observe(0, 0, start, start);
    // A sender backlog: video arrives 800 ms late and keeps playing.
    let mut seq = 1u16;
    let mut frame = |edge: &mut LiveEdge, captured: u64| {
        let arrived = after(captured + 800);
        assert!(!edge.observe(ts(captured), seq, arrived, arrived));
        seq = seq.wrapping_add(1);
        arrived
    };
    for captured in (16..=2_000).step_by(16) {
        let arrived = frame(&mut edge, captured);
        assert!(edge.can_present(ts(captured), arrived));
    }
    // Loss breaks the chain: an incident, and one request at once.
    edge.damage();
    let sent = after(2_800);
    assert!(matches!(
        edge.keyframe_request(false, None, sent),
        Request::Send(_)
    ));
    // The next backoff slot (300 ms) is held until the keyframe could have
    // come through the 800 ms backlog: lateness + cooldown.
    let mut next = None;
    for captured in (2_016..=3_400).step_by(16) {
        let now = frame(&mut edge, captured);
        match edge.keyframe_request(false, Some(sent), now) {
            Request::Send(_) => {
                next = Some(now - sent);
                break;
            }
            Request::Wait => assert!(now - sent < Duration::from_millis(300)),
            Request::Suppressed(reason) => assert_eq!(reason, Suppression::InFlight),
            Request::Idle => panic!("idle while recovering"),
        }
    }
    let next = next.expect("requests stopped");
    assert!(
        next >= REQUEST_COOLDOWN + Duration::from_millis(800),
        "{next:?}"
    );
    assert!(
        next < REQUEST_COOLDOWN + Duration::from_millis(816),
        "{next:?}"
    );
    // The keyframe starts arriving 800 ms late and takes 290 ms to arrive: it
    // is judged by its first packet, admitted, and recovers.
    let first = after(2_810 + 800);
    edge.observe(ts(2_810), seq, first, first);
    let done = first + Duration::from_millis(290);
    assert!(edge.admit(ts(2_810), true, done));
    edge.submitted(ts(2_810), true, done);
    assert!(edge.presented(ts(2_810), done + Duration::from_millis(20)));
    assert_eq!(
        (edge.state(), edge.incidents, edge.recovered),
        (State::Live, 1, 1)
    );
}
