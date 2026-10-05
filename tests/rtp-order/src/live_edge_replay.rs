//! Actual returned timing observations, fed to production media-clock policy.
//! No payload exists here: never claim to replay decoding or sender recovery.
use crate::live_edge::{LAG_CEILING, LiveEdge, MediaClock, State};
use std::time::{Duration, Instant};

#[test]
fn returned_diag03_8_onset_plays_late_video_and_preserves_current_audio() {
    let start = Instant::now();
    let mut video = LiveEdge::default();
    let mut audio = MediaClock::new(48_000);
    let mut requests = 0;
    let mut late_shown = 0;
    let mut audio_max = Duration::ZERO;
    let mut video_max = 0;
    let mut video_packets = 0;
    let mut audio_packets = 0;
    for line in include_str!("../fixtures/diag03-8-onset.csv")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .skip(1)
    {
        let fields: Vec<u64> = line.split(',').map(|v| v.parse().unwrap()).collect();
        let d = start + Duration::from_micros(fields[1]);
        let l = start + Duration::from_micros(fields[2]);
        let ts = fields[3] as u32;
        let seq = fields[4] as u16;
        if fields[0] == 1 {
            video_packets += 1;
            // This retained window begins well after successful playback in the
            // original device history. Seed its first observed playable point;
            // the fixture has no payload/render events to manufacture startup.
            if video_packets == 1 {
                video.establish(ts, d, d);
            }
            assert!(!video.observe(ts, seq, d, l), "incident at {}us", fields[1]);
            if video.request_due(l) {
                requests += 1;
            }
            let lag = video.added_delay_ms(ts, l).unwrap();
            video_max = video_max.max(lag);
            // HA06: the 1.6 s sender backlog is decoded and shown late, not
            // frozen; nothing is discarded and no keyframe is needed.
            assert!(
                video.admit(ts, false, l),
                "late video rejected at {}us",
                fields[1]
            );
            assert!(video.can_present(ts, l));
            if lag > 240 {
                late_shown += 1;
            }
        } else {
            audio_packets += 1;
            audio.observe(ts, seq, d);
            let lag = audio.delay(ts, l).unwrap();
            audio_max = audio_max.max(lag);
        }
    }
    assert!(video_max >= 1595);
    assert!(Duration::from_millis(video_max) < LAG_CEILING);
    assert!(audio_max < Duration::from_millis(240));
    assert_eq!(video.state(), State::Live);
    assert_eq!((video.incidents, video.recovered, requests), (0, 0, 0));
    println!(
        "DIAG03-8: video packets={video_packets}, audio packets={audio_packets}, max relative video={video_max}ms (ceiling {}ms), shown more than 240 ms late={late_shown}, audio={}us, incidents 0, requests 0; whether the sender then drained is UNPROVEN (no payload/counterfactual sender)",
        LAG_CEILING.as_millis(),
        audio_max.as_micros()
    );
}
