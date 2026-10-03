//! Actual returned timing observations, fed to production media-clock policy.
//! No payload exists here: never claim to replay decoding or sender recovery.
use crate::live_edge::{
    BACKLOG, LiveEdge, MediaClock, REQUEST_COOLDOWN, Request, State, Suppression,
};
use std::time::{Duration, Instant};

#[test]
fn returned_diag03_8_onset_blocks_stale_video_and_preserves_current_audio() {
    let start = Instant::now();
    let mut video = LiveEdge::default();
    let mut audio = MediaClock::new(48_000);
    let mut incident = None;
    let mut incident_lag = None;
    let mut rejected = 0;
    let mut requests = Vec::new();
    let mut last_video = start;
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
            if video.observe(ts, seq, d, l) {
                incident.get_or_insert(fields[1]);
                incident_lag.get_or_insert(video.added_delay_ms(ts, l).unwrap());
            }
            if video.request_due(l) {
                requests.push((l, video.added_delay_ms(ts, d).unwrap()));
            }
            last_video = l;
            let lag = video.added_delay_ms(ts, l).unwrap();
            video_max = video_max.max(lag);
            if !video.admit(ts, true, l) {
                rejected += 1;
            }
            if fields[1] > 116_000_000 {
                assert!(
                    !video.admit(ts, true, l),
                    "old video passed at {}us",
                    fields[1]
                );
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
    // Payloads are absent, so no real IDR can be submitted/confirmed here.
    // HA05: requests go out only while video arrives less than BACKLOG late
    // (it was briefly current again 0.3-0.7 s after the onset), and none into
    // the plateau that follows: each keyframe would queue behind it (HA04
    // sent 12 here). They are deferred, never abandoned.
    let times: Vec<_> = requests.iter().map(|(at, _)| *at).collect();
    let gaps: Vec<_> = times.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.iter().all(|gap| *gap >= REQUEST_COOLDOWN), "{gaps:?}");
    assert!(
        requests
            .iter()
            .all(|(_, late)| *late < BACKLOG.as_millis() as u64),
        "requested into late video: {requests:?}"
    );
    let onset = start + Duration::from_micros(incident.unwrap());
    assert!(
        (1..=2).contains(&requests.len())
            && times.iter().all(|at| *at - onset < Duration::from_secs(1)),
        "requested into the stale plateau"
    );
    assert_eq!(
        video.pending_request(last_video),
        Request::Suppressed(Suppression::Backlog)
    );
    let window = last_video - *times.last().unwrap();
    println!(
        "DIAG03-8: video packets={video_packets}, audio packets={audio_packets}, quarantine D={}us at {}ms, max relative video={video_max}ms, audio={}us, rejected={rejected}, requests={} while video was current, then none for the {}ms plateau; actual live recovery=UNPROVEN (no payload/counterfactual sender)",
        incident.unwrap(),
        incident_lag.unwrap(),
        audio_max.as_micros(),
        requests.len(),
        window.as_millis()
    );
}
