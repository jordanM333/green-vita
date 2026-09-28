//! Actual returned timing observations, fed to production media-clock policy.
//! No payload exists here: never claim to replay decoding or sender recovery.
use crate::live_edge::{LiveEdge, MediaClock, State};
use std::time::{Duration, Instant};

#[test]
fn returned_diag03_8_onset_blocks_stale_video_and_preserves_current_audio() {
    let start = Instant::now();
    let mut video = LiveEdge::default();
    let mut audio = MediaClock::new(48_000);
    let mut incident = None;
    let mut incident_lag = None;
    let mut rejected = 0;
    let mut requests = 0;
    let mut audio_max = Duration::ZERO;
    let mut video_max = 0;
    let mut video_packets = 0;
    let mut audio_packets = 0;
    for line in include_str!("../fixtures/diag03-8-onset.csv").lines().filter(|l| !l.starts_with('#')).skip(1) {
        let fields: Vec<u64> = line.split(',').map(|v| v.parse().unwrap()).collect();
        let d = start + Duration::from_micros(fields[1]);
        let l = start + Duration::from_micros(fields[2]);
        let ts = fields[3] as u32;
        let seq = fields[4] as u16;
        if fields[0] == 1 {
            video_packets += 1;
            if video.observe(ts, seq, d, l) {
                incident.get_or_insert(fields[1]);
                incident_lag.get_or_insert(video.added_delay_ms(ts, l).unwrap());
            }
            requests += u32::from(video.request_due(l));
            let lag = video.added_delay_ms(ts, l).unwrap();
            video_max = video_max.max(lag);
            if !video.admit(ts, true, l) { rejected += 1; }
            if fields[1] > 116_000_000 {
                assert!(!video.admit(ts, true, l), "old video passed at {}us", fields[1]);
                assert!(!video.can_present(ts, l));
            }
        } else {
            audio_packets += 1;
            audio.observe(ts, seq, d);
            let lag = audio.delay(ts, l).unwrap();
            audio_max = audio_max.max(lag);
            // Includes the existing 120ms maximum output queue and 20ms buffer.
            assert!(l + Duration::from_millis(140) <= audio.deadline(ts).unwrap());
        }
    }
    assert!(incident.is_some_and(|at| (113_000_000..115_500_000).contains(&at)));
    assert!(incident_lag.is_some_and(|lag| (240..480).contains(&lag)));
    assert!(video_max >= 1595);
    assert!(audio_max < Duration::from_millis(240));
    assert_eq!(video.state(), State::AwaitingKeyframe);
    assert_eq!(video.recovered, 0);
    assert_eq!(video.incidents, 1);
    // Up to three initial requests plus one bounded rearm if current timestamps
    // return. Payloads are absent, so we cannot submit/confirm a real IDR here.
    assert!((3..=6).contains(&requests));
    println!("DIAG03-8: video packets={video_packets}, audio packets={audio_packets}, quarantine D={}us at {}ms, max relative video={video_max}ms, audio={}us, rejected={rejected}, requests={requests}; actual live recovery=UNPROVEN (no payload/counterfactual sender)", incident.unwrap(), incident_lag.unwrap(), audio_max.as_micros());
}
