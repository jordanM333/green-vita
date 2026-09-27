use super::*;

fn sample(at: u64, stage: &'static str, id: Identity, a: u64, b: u64, c: u64) -> Event {
    Event {
        at,
        stage,
        epoch: 1,
        id,
        a,
        b,
        c,
    }
}
fn replay(kind: &str) -> Capture {
    let mut capture = Capture::new(Instant::now());
    let mut events = Vec::new();
    for n in 0..1800u64 {
        let media = n * 1_000_000 / 60 + 1_000_000;
        let deficit = media.saturating_sub(6_000_000).min(4_000_000) / 2;
        let extra = if kind == "healthy" { 0 } else { deficit };
        let dequeued = media + if kind == "rtc" { 0 } else { extra };
        let delivered = dequeued + 200 + if kind == "rtc" { extra } else { 0 };
        let id = Identity {
            ssrc: 7,
            timestamp: (u32::MAX - 90_000).wrapping_add((n * 1500) as u32),
            sequence: 65500u16.wrapping_add(n as u16),
            media: 1,
            flags: 7,
        };
        let empty = match kind {
            "ambiguous" => 0,
            "unknown" => UNKNOWN,
            _ => dequeued - 1500,
        };
        events.push(sample(
            dequeued,
            "udp",
            Identity {
                media: 0,
                flags: 3,
                ..id
            },
            dequeued - 3,
            empty,
            800,
        ));
        events.push(sample(delivered, "rtc", id, dequeued, UNKNOWN, 90_000));
        events.push(sample(
            delivered + 200,
            "ordered",
            id,
            dequeued,
            delivered,
            90_000,
        ));
        events.push(sample(
            delivered + 400,
            "au_complete",
            Identity {
                timestamp: id.timestamp,
                ..Identity::default()
            },
            delivered + 400 - dequeued,
            UNKNOWN,
            UNKNOWN,
        ));
        events.push(sample(
            delivered + 450,
            "h264_assembly_us",
            Identity {
                timestamp: id.timestamp,
                ..Identity::default()
            },
            200,
            UNKNOWN,
            UNKNOWN,
        ));
        events.push(sample(
            delivered + 500,
            "decode_submit",
            Identity {
                timestamp: id.timestamp,
                ..Identity::default()
            },
            delivered + 500 - dequeued,
            UNKNOWN,
            UNKNOWN,
        ));
        if n % 60 == 0 {
            events.push(sample(
                delivered + 700,
                "receiver_ceiling_bps",
                Identity::default(),
                500_000,
                UNKNOWN,
                UNKNOWN,
            ));
        }
    }
    // Independent, current 20ms audio on the same socket with timestamp wrap.
    for n in 0..1700u64 {
        let at = 1_000_000 + n * 20_000;
        let id = Identity {
            ssrc: 8,
            timestamp: (u32::MAX - 48_000).wrapping_add((n * 960) as u32),
            sequence: 65500u16.wrapping_add(n as u16),
            media: 2,
            flags: 5,
        };
        events.push(sample(
            at,
            "udp",
            Identity {
                media: 0,
                flags: 1,
                ..id
            },
            at - 3,
            at - 1500,
            100,
        ));
        events.push(sample(at + 200, "rtc", id, at, UNKNOWN, 48_000));
    }
    events.sort_by_key(|e| e.at);
    let mut last_snapshot = 0;
    for event in events {
        capture.push(event);
        if event.at > last_snapshot + 1_000_000 {
            capture.snapshot(
                event.at,
                "Build: host fixture\nMode:Home\nMic:OFF\nSocket test summary",
            );
            last_snapshot = event.at;
        }
    }
    capture.finish(181_000_000);
    capture
}

#[test]
fn first_incident_survives_delayed_exit_and_a_second_stream() {
    let mut capture = replay("upstream");
    assert_eq!(capture.reason, "INCIDENT CAPTURED");
    let before = (
        capture.initial.len(),
        capture.pre.len(),
        capture.post.len(),
        capture.trigger,
        capture.frozen,
    );
    assert!(capture.trigger.unwrap() < 8_000_000);
    assert!(
        capture
            .initial
            .iter()
            .any(|e| e.stage == "rtc" && e.at < 2_000_000)
    );
    assert!(capture.post.iter().any(|e| e.id.media == 2));
    for n in 0..1_000_000u64 {
        let mut e = sample(
            100_000_000 + n * 1000,
            "rtc",
            Identity {
                media: 1,
                flags: 5,
                ..Default::default()
            },
            0,
            0,
            90_000,
        );
        e.epoch = 2;
        capture.push(e);
    }
    capture.finish(2_000_000_000);
    assert_eq!(
        before,
        (
            capture.initial.len(),
            capture.pre.len(),
            capture.post.len(),
            capture.trigger,
            capture.frozen
        )
    );
}
#[test]
fn allocation_budget_and_capacity_overflow_are_explicit() {
    assert!(std::mem::size_of::<Event>() <= 64);
    let mut capture = Capture::new(Instant::now());
    capture.first_video = Some(0);
    capture.trigger = Some(5_000_000);
    for n in 0..CAP + 1 {
        capture.push(sample(
            6_000_000 + n as u64,
            "udp",
            Default::default(),
            0,
            0,
            100,
        ));
    }
    assert_eq!(capture.post.len(), CAP);
    assert_eq!(capture.truncated, 1);
    assert_eq!(capture.reason, "CAPTURE INCOMPLETE: capacity");
}
#[test]
fn trigger_ignores_duplicate_late_packets_wrap_and_resets() {
    let mut clock = TriggerClock::default();
    for n in 0..1200u64 {
        let e = sample(
            n * 1_000_000 / 60,
            "rtc",
            Identity {
                ssrc: 1,
                media: 1,
                flags: 5,
                timestamp: (u32::MAX - 90_000).wrapping_add((n * 1500) as u32),
                ..Default::default()
            },
            0,
            0,
            90_000,
        );
        assert!(!clock.receive(e));
        assert!(!clock.receive(e));
        let mut late = e;
        late.id.timestamp = late.id.timestamp.wrapping_sub(1500);
        assert!(!clock.receive(late));
    }
    let reset = sample(
        25_000_000,
        "rtc",
        Identity {
            ssrc: 2,
            timestamp: 123,
            media: 1,
            flags: 5,
            ..Default::default()
        },
        0,
        0,
        90_000,
    );
    assert!(!clock.receive(reset));
}
#[test]
fn timeout_early_exit_and_missing_video_do_not_claim_fix() {
    let healthy = replay("healthy");
    assert_eq!(healthy.reason, "NOT REPRODUCED DURING CAPTURE");
    let mut early = Capture::new(Instant::now());
    early.trigger = Some(200);
    early.finish(1000);
    assert_eq!(early.reason, "CAPTURE INCOMPLETE: early exit");
    let mut late = Capture::new(Instant::now());
    late.first_video = Some(0);
    late.trigger = Some(179_000_000);
    late.tick(181_000_000);
    assert!(late.frozen.is_none());
    late.tick(190_000_000);
    assert_eq!(late.reason, "INCIDENT CAPTURED");
}
#[test]
fn real_capture_save_and_immutable_export_fixtures() {
    let Some(root) = std::env::var_os("GREENVITA_DIAGNOSTIC_FIXTURES") else {
        return;
    };
    let root = PathBuf::from(root);
    std::fs::create_dir_all(&root).unwrap();
    for kind in ["upstream", "rtc", "ambiguous", "unknown", "healthy"] {
        let capture = replay(kind);
        let destination = root.join(kind);
        // These are test-owned destinations, never the device capture path.
        if destination.exists() {
            std::fs::remove_dir_all(&destination).unwrap();
        }
        capture.save_to(&destination, 0).unwrap();
        let bytes = std::fs::read(destination.join("events.csv")).unwrap();
        let other = replay("healthy");
        other.save_to(&destination, 99).unwrap();
        assert_eq!(
            std::fs::read(destination.join("events.csv")).unwrap(),
            bytes
        );
    }
}
#[test]
fn raw_control_and_short_headers_are_not_trusted_media() {
    assert_eq!(udp_identity(&[]).flags, 0);
    assert_eq!(
        udp_identity(&[0x80, 200, 0, 1, 0, 0, 0, 1, 0, 0, 0, 7]).flags,
        0
    );
    let id = udp_identity(&[0x80, 0xe0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 7]);
    assert_eq!(id.ssrc, 7);
    assert_eq!(id.sequence, 65535);
    assert_eq!(id.timestamp, u32::MAX);
    assert_eq!(id.media, 0);
    assert_eq!(id.flags, 3);
}

#[test]
fn public_recording_hooks_keep_original_instants_and_measure_host_overhead() {
    let start = Instant::now();
    *CAPTURE.lock().unwrap() = Some(Capture::new(start));
    STATE.store(1, Ordering::Relaxed);
    let original = start + std::time::Duration::from_millis(10);
    let delivered = start + std::time::Duration::from_millis(20);
    let id = Identity {
        ssrc: 9,
        timestamp: 42,
        sequence: 65535,
        media: 1,
        flags: 5,
    };
    packet("rtc", id, delivered, Some(original), None, 90_000);
    {
        let guard = CAPTURE.lock().unwrap();
        let c = guard.as_ref().unwrap();
        let e = c.pre.back().unwrap();
        assert_eq!(e.at, 20_000);
        assert_eq!(e.a, 10_000);
        assert_eq!(e.b, UNKNOWN);
        assert_eq!(e.id.ssrc, 9);
        // A contended probe returns immediately and records loss, never waits on the lock.
        event("lock_contention", 0, 0);
        assert!(SKIPPED.load(Ordering::Relaxed) > 0);
    }
    let measured = Instant::now();
    for n in 0..1_000_000u32 {
        event("host_overhead", n, 1);
    }
    let ns = measured.elapsed().as_nanos();
    println!(
        "Host-only recorder benchmark: 1000000 calls, {} ns/call; Vita overhead UNMEASURED",
        ns / 1_000_000
    );
    STATE.store(0, Ordering::Relaxed);
    *CAPTURE.lock().unwrap() = None;
}

#[test]
fn failed_save_preserves_existing_partial_evidence() {
    let root = std::env::temp_dir().join(format!(
        "gv-partial-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("capture");
    let partial = destination.with_extension("partial");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("events.csv"), "first incident").unwrap();
    let capture = replay("upstream");
    assert!(capture.save_to(&destination, 0).is_err());
    assert_eq!(
        std::fs::read_to_string(partial.join("events.csv")).unwrap(),
        "first incident"
    );
    std::fs::remove_dir_all(root).unwrap();
}
