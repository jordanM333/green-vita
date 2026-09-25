use super::*;
fn packet(at: Instant) -> TimedAudio<Bytes> {
    // Valid 20ms stereo Opus silence; decoded by the installed native libopus.
    TimedAudio { data: Bytes::from_static(&[0xf8, 0xff, 0xfe]), received_at: at }
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
    for _ in 0..32 { tx.send(packet(Instant::now())).unwrap(); }
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
    let mut renderer = AudioRenderer::new(&sdl2::AudioSubsystem).unwrap();
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
    let mut renderer = AudioRenderer::new(&sdl2::AudioSubsystem).unwrap();
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
        let mut renderer = AudioRenderer::new(&sdl2::AudioSubsystem).unwrap();
        renderer.submit_packets(vec![packet(Instant::now())], 50);
        renderer.reset_stream();
        assert_eq!(renderer.queue.size(), 0);
        assert_eq!(METRICS.audio_pcm_pending.load(Ordering::Relaxed), 0);
    }
}
