use super::*;
fn packet(at: Instant) -> TimedAudio<Bytes> {
    // Valid 20ms stereo Opus silence; decoded by the installed native libopus.
    TimedAudio {
        data: Bytes::from_static(&[0xf8, 0xff, 0xfe]),
        received_at: at,
    }
}
fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready() {
        assert!(Instant::now() < deadline, "audio worker stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn full_pcm_sink_does_not_block_prediction_or_shutdown() {
    let (tx, rx, thread) = spawn_decode_worker().unwrap();
    METRICS.audio_opus_pending.store(32, Ordering::Relaxed);
    for _ in 0..32 {
        tx.send(packet(Instant::now())).unwrap();
    }
    // Do not consume PCM. The old blocking send stopped after eight buffers.
    wait_for(|| METRICS.audio_opus_pending.load(Ordering::Relaxed) == 0);
    drop(tx);
    wait_for(|| thread.is_finished());
    thread.join().unwrap();
    assert_eq!(rx.try_iter().count(), MAX_PENDING_PCM_BUFFERS);
    METRICS.audio_pcm_pending.store(0, Ordering::Relaxed);
}
#[test]
fn old_audio_never_reaches_device_after_six_second_consumer_stall() {
    let sdl = sdl2::init().unwrap();
    let audio = sdl.audio().unwrap();
    let mut renderer = AudioRenderer::new(&audio).unwrap();
    let before = METRICS.audio_pcm_discarded.load(Ordering::Relaxed);
    renderer.submit_packets(vec![packet(Instant::now() - Duration::from_secs(6))], 100);
    wait_for(|| METRICS.audio_pcm_discarded.load(Ordering::Relaxed) > before);
    renderer.submit_packets(vec![], 100);
    assert_eq!(renderer.queue.size(), 0);
    renderer.submit_packets(vec![packet(Instant::now())], 100);
    wait_for(|| METRICS.audio_pcm_pending.load(Ordering::Relaxed) > 0);
    renderer.submit_packets(vec![], 100);
    assert_eq!(renderer.queue.size(), 3840, "current 20ms frame must play");
}
#[test]
fn already_decoded_pcm_expires_in_the_handoff_too() {
    let sdl = sdl2::init().unwrap();
    let audio = sdl.audio().unwrap();
    let mut renderer = AudioRenderer::new(&audio).unwrap();
    renderer.submit_packets(vec![packet(Instant::now())], 100);
    wait_for(|| METRICS.audio_pcm_pending.load(Ordering::Relaxed) > 0);
    // Model elapsed wall time without a multi-second sleep: preserve the
    // actual decoded samples and inject an aged handoff into the real renderer.
    let mut pcm = renderer.samples_rx.recv().unwrap();
    pcm.received_at -= Duration::from_secs(6);
    let (tx, rx) = sync_channel(1);
    tx.send(pcm).unwrap();
    let original = std::mem::replace(&mut renderer.samples_rx, rx);
    renderer.submit_packets(vec![], 100);
    assert_eq!(renderer.queue.size(), 0);
    renderer.samples_rx = original;
}
#[test]
fn repeated_start_stop_joins_workers_and_clears_output() {
    for _ in 0..100 {
        let sdl = sdl2::init().unwrap();
        let audio = sdl.audio().unwrap();
        let mut renderer = AudioRenderer::new(&audio).unwrap();
        renderer.submit_packets(vec![packet(Instant::now())], 50);
        renderer.reset_stream();
        assert_eq!(renderer.queue.size(), 0);
        assert_eq!(METRICS.audio_pcm_pending.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn opus_rejects_undersized_output_before_native_write() {
    let mut decoder = NativeOpusDecoder::new().unwrap();
    let mut pcm = [123i16; 1];
    assert!(
        decoder
            .decode(&[0xf8, 0xff, 0xfe], &mut pcm)
            .unwrap_err()
            .to_string()
            .contains("too small")
    );
    assert_eq!(pcm, [123]);
}

#[test]
fn thirty_minutes_of_pcm_overproduction_cannot_build_a_device_archive() {
    // 90,000 20ms PCM frames = 30 media minutes, accelerated. The recording SDL
    // sink consumes nothing: this stresses the real trim/admission policy, not
    // the Vita DAC or its clock. Native Opus correctness is tested separately.
    let sdl = sdl2::init().unwrap();
    let audio = sdl.audio().unwrap();
    let mut renderer = AudioRenderer::new(&audio).unwrap();
    let (tx, rx) = sync_channel(1);
    let original = std::mem::replace(&mut renderer.samples_rx, rx);
    let before = METRICS.audio_latency_trims.load(Ordering::Relaxed);
    let mut maximum = 0;
    for tick in 0..90_000 {
        METRICS.audio_pcm_pending.fetch_add(1, Ordering::Relaxed);
        tx.send(TimedAudio {
            data: vec![0; 1920],
            received_at: Instant::now()
                - if tick % 1000 == 0 {
                    Duration::from_secs(6)
                } else {
                    Duration::ZERO
                },
        })
        .unwrap();
        renderer.submit_packets(vec![], 100);
        maximum = maximum.max(renderer.queue.size());
        assert!(renderer.queue.size() <= AUDIO_TRIM_THRESHOLD_BYTES);
    }
    assert!(METRICS.audio_latency_trims.load(Ordering::Relaxed) > before);
    renderer.samples_rx = original;
    renderer.reset_stream();
    assert_eq!(renderer.queue.size(), 0);
    println!(
        "30 media minutes of overproduction: maximum software output {}ms; reset leaves zero queued bytes",
        u64::from(maximum) * 1000 / u64::from(AUDIO_BYTES_PER_SECOND)
    );
}

#[test]
fn sample_clock_mismatch_remains_bounded_and_recovery_discards_old_pcm() {
    // Explicit sample-clock simulation, not a measurement of the Vita DAC.
    // Both clock directions matter: a slow device accumulates samples; a fast
    // device underruns. Run the production admission/start/trim policy for
    // 30 media minutes per direction, retaining fractional sample consumption.
    for ppm in [-1000i64, 1000] {
        let sdl = sdl2::init().unwrap();
        let audio = sdl.audio().unwrap();
        let mut renderer = AudioRenderer::new(&audio).unwrap();
        let (tx, rx) = sync_channel(1);
        let original = std::mem::replace(&mut renderer.samples_rx, rx);
        let trims = METRICS.audio_latency_trims.load(Ordering::Relaxed);
        let underruns = METRICS.audio_underruns.load(Ordering::Relaxed);
        let mut fraction = 0i64;
        let mut maximum = 0;
        for _ in 0..90_000 {
            if renderer.started {
                fraction += 960 * (1_000_000 + ppm);
                let frames = fraction / 1_000_000;
                fraction %= 1_000_000;
                let mut queued = renderer.queue.samples.borrow_mut();
                let consumed = queued.len().min(frames as usize * 2);
                queued.drain(..consumed);
            }
            METRICS.audio_pcm_pending.fetch_add(1, Ordering::Relaxed);
            tx.send(TimedAudio {
                data: vec![7; 1920],
                received_at: Instant::now(),
            })
            .unwrap();
            renderer.submit_packets(vec![], 100);
            maximum = maximum.max(renderer.queue.size());
            assert!(renderer.queue.size() <= AUDIO_TRIM_THRESHOLD_BYTES);
        }
        if ppm < 0 {
            assert!(METRICS.audio_latency_trims.load(Ordering::Relaxed) > trims);
        } else {
            assert!(METRICS.audio_underruns.load(Ordering::Relaxed) > underruns);
        }
        renderer.samples_rx = original;
        renderer.reset_stream();
        assert_eq!(renderer.queue.size(), 0);
        assert_eq!(METRICS.audio_pcm_pending.load(Ordering::Relaxed), 0);
        println!(
            "30 media minutes, DAC model {ppm:+}ppm: software queue maximum {}ms; recovery empty",
            u64::from(maximum) * 1000 / u64::from(AUDIO_BYTES_PER_SECOND)
        );
    }
}
