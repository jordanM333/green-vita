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
    // HA05: video kept arriving 1.6 s late for the whole plateau, so nothing
    // was requested into it: a keyframe would queue behind it (HA04-20). The
    // request is deferred, not abandoned.
    assert!(
        requests.is_empty(),
        "{} requests into a backlog",
        requests.len()
    );
    assert_eq!(
        edge.pending_request(last),
        Request::Suppressed(Suppression::Backlog)
    );
    assert_eq!(edge.state(), State::AwaitingKeyframe);
    // Actual current media returns 100 ms later. Dependent pictures alone
    // cannot restart; the first request goes out at once.
    let current = last + Duration::from_millis(100);
    let ts = (current.duration_since(start).as_micros() * 90 / 1000) as u32;
    edge.observe(ts, 0, current, current);
    assert!(!edge.admit(ts, false, current));
    assert!(edge.request_due(current), "current media restarts requests");
    assert!(edge.admit(ts, true, current));
    edge.submitted(ts, true, current);
    assert_eq!(edge.recovered, 0, "admission is not presentation");
    assert!(edge.presented(ts, current + Duration::from_millis(80)));
    assert_eq!(edge.state(), State::Live);
    assert_eq!(edge.recovered, 1);
    println!(
        "1595ms model: quarantine at {}us; 30min plateau blocked with no keyframe requested into it; current media: one request, current-IDR output restores LIVE",
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
    // Silence (no new media), not a backlog: requests follow the backoff.
    let now = start + Duration::from_secs(2);
    assert!(edge.poll(now));
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

#[test]
fn a_recovery_keyframe_is_judged_by_its_first_packet_and_may_catch_up_for_one_second() {
    let at = |ms: u64| Instant::now() + Duration::from_millis(ms);
    let start = at(0);
    let after = |ms: u64| start + Duration::from_millis(ms);
    let ts = |ms: u32| ms * 90;
    let mut edge = LiveEdge::default();
    edge.establish(0, start, start);
    edge.observe(0, 0, start, start);
    // Silence: an incident, and a request on the HA04 schedule.
    assert!(edge.poll(after(1_000)));
    assert!(edge.request_due(after(1_000)));
    // The keyframe captured at 1.1 s starts arriving 100 ms late, but takes
    // 230 ms to transmit: its last packet is 330 ms late.
    edge.observe(ts(1_100), 1, after(1_200), after(1_200));
    assert!(edge.ingress_useful(ts(1_100), after(1_430)));
    assert!(edge.admit(ts(1_100), true, after(1_430)));
    edge.submitted(ts(1_100), true, after(1_430));
    assert_eq!(edge.state(), State::AwaitingPicture);
    // Frames queued behind it arrive up to MEDIA_BUDGET late: accepted, and
    // no new incident, while the allowance lasts.
    let mut seq = 2;
    for (captured, arrived) in [(1_117, 1_530), (1_133, 1_540), (1_150, 1_545)] {
        assert!(!edge.observe(ts(captured), seq, after(arrived), after(arrived)));
        assert!(edge.ingress_useful(ts(captured), after(arrived)));
        seq += 1;
    }
    assert!(edge.presented(ts(1_117), after(1_560)));
    assert_eq!(edge.state(), State::Live);
    for step in 0..50u32 {
        let captured = 1_167 + step * 16;
        assert!(!edge.observe(
            ts(captured),
            seq,
            after(u64::from(captured) + 300),
            after(u64::from(captured) + 300)
        ));
        seq += 1;
    }
    assert_eq!(
        (edge.incidents, edge.caught_up, edge.catch_up_late),
        (1, 0, 0)
    );
    // Beyond MEDIA_BUDGET the allowance does not apply.
    assert!(!edge.ingress_useful(ts(1_900), after(2_400)));
    // The allowance ends 1 s after admission. Still 300 ms late: the usual
    // rule resumes and confirms a new incident after CONFIRMATION.
    let mut incident = None;
    for step in 0..20u32 {
        let captured = 2_000 + step * 16;
        let arrived = after(u64::from(captured) + 300);
        if edge.observe(ts(captured), seq, arrived, arrived) {
            incident = Some(u64::from(captured) + 300);
            break;
        }
        seq += 1;
    }
    assert_eq!((edge.caught_up, edge.catch_up_late), (0, 1));
    let incident = incident.expect("late video after the allowance is an incident");
    // Pressure starts at the first frame after the allowance (2444 ms).
    assert!(
        (2_430 + 120..2_430 + 120 + 40).contains(&incident),
        "{incident}"
    );
    assert_eq!(edge.incidents, 2);
}
