# HA07: live video — stop slowing the Xbox's video, and catch up through drains

This candidate is a hypothesis fix for HA06-22's lag behind controls. It is NOT
confirmed until a physical Vita shows it. HA06-22's status reports Mode:Home
again, so Cloud sessions have still not been captured showing this pattern.
Rollback source is 5998ce1887689e8f474d8df9f73af7888907dbef (HA06-22).

The user's report with HA06-22: "The video does catch up now, but is now still
lagging behind actual input controls. What can be next to finally solve the
issues?"

## What HA06-22 showed (Home session; trigger 117.3 s, capture to 127.3 s, incident log to 195.5 s)

* **85-117 s was healthy.** 59-63 fps shown, video payload 1.1-1.4 Mbps
  averaged per second, video and audio sender-report offsets within 10 ms of
  each other, and REMB 2000k.
* **A heavier scene started at about 117.4 s.** Video payload rose to
  1.8-2.6 Mbps (peaks 2.1-3.3 Mbps). Video's added delay rose to 607 ms and
  then 1104 ms, while audio stayed current.
* **The receiver read that video lateness as congestion and cut the REMB.**
  2000k -> 1400k at 118.8 s (video delay 1286 ms), 980k at 119.9 s, 686k at
  121.0 s and 500k at 125.4 s (1755 ms).
* **At 500k the Xbox paced video at about 0.93 Mbps, below moderate gameplay.**
  Its queue then sat about 1.3 s deep, as libwebrtc's 2 s queue-time limit
  predicts. The REMB recovers only after 10 s at 240 ms or less, which never
  came. Video stayed 1.0-1.9 s behind with audio current: the lag behind the
  controls.
* **Drains froze the picture.** When the Xbox caught up, it delivered up to
  2.7 s of video per second. AVCDEC completes about 95 AUs per second, so AUs
  waited 240-330 ms for it. Between 153 and 195 s:
  * 225 pictures expired at the 240 ms local limit, with 37 holds and picture
    gaps of 0.3-0.73 s;
  * at 195.2 s the 32-AU queue overflowed (`queue_frame_limit 32`) and forced
    a keyframe recovery.
* **HA04-20 agrees on the sender.** At REMB 2000k it paced about
  2.0-2.1 Mbps, while its encoder produced 2.4-4.4 Mbps in heavy scenes.

## Root cause

1. **The REMB treated lateness inside the Xbox as path congestion.** Audio and
   video share one BUNDLE flow. Audio stayed current, so the delay was in the
   Xbox's own pacer queue, which sends audio first. Lowering the REMB slowed
   that pacer, but the encoder did not shrink its frames. So the backlog
   persisted, at about 1.3 s for over a minute.
2. **A 2 Mbps ceiling is below heavy scenes.** The Xbox paces at about its
   estimate, which our REMB caps. Its encoder follows content, not the REMB.
   So even without cuts, scenes above about 2 Mbps queue.
3. **Drains hit the 240 ms local limit.** AUs and pictures waiting for AVCDEC
   during a drain expired. That froze the screen and forced keyframe waits.

## Changes

| Change | Where |
| --- | --- |
| Only delay audio shares lowers the REMB. The budget's input is the lower of video's added delay and the latest audio packet's added delay, if audio arrived in the last second, and 0 otherwise (no evidence about the path). Status shows both: `delay:` (video) and `path:` (shared). | feedback.rs `shared_path_delay_ms`, `VideoCeiling::receive`; clock.rs `recent_delay_ms`; session.rs |
| SDP and REMB video ceiling 2 -> 3 Mbps (`b=AS:3000`, `b=TIAS:3000000`). The REMB starts at, and recovers toward, 3000k. The control channel's `maxBitrateKbps` startup hint stays 2000. | bandwidth.rs |
| Local catch-up. An AU is decoded, and its picture shown, until 2.5 s (`MAX_LOCAL_CATCH_UP`, equal to `LAG_CEILING`) after the AU completed, instead of 240 ms. A picture stays live while the decoder produced it within 240 ms, so the hold ("Reconnecting video…") starts 240 ms after the last decoded picture. Picture selection now uses the same limits as drawing. | policy.rs, worker.rs, video/mod.rs |
| AU queue 32 -> 128 AUs. The 4 MB byte budget is unchanged. | policy.rs |
| Diagnostics: label, build number HA07, schema notes. | diagnostic.rs, final-build.yml, build_provenance.py, DIAGNOSTIC-SCHEMA.txt |

## Call-outs: constrained settings that HA07 changes

The standing instruction is not to change bitrate, resolution, receive budgets,
thread priorities, audio policy or SDP ceilings unless the task requires it.
HA06-22 shows that two of them cause the lag, so HA07 changes them:

* **SDP ceiling and REMB maximum: 2 Mbps -> 3 Mbps.**
* **The receive budget's input:** only delay audio shares can lower the REMB.
  The budget's own rules are unchanged: x0.7 cuts after two +20 ms windows,
  the 500 kbps floor, and +100 kbps per 10 s.

Two local Vita-side limits also change: the AU and picture age limit
(240 ms -> 2.5 s while catching up) and the AU queue (32 -> 128).

Unchanged: the resolution request (1280x720), the `maxBitrateKbps` hint
(2000), thread priorities, audio policy and its 480 ms deadline, receive-pass
budgets, `LAG_CEILING`, keyframe request policy, the decoder, texture and GXM
path, the hold indicator, Home sign-in handling, and the DISPLAY01 and latency
capture formats.

## Trade-offs

* **The Xbox may send up to about 3 Mbps of video.** If Wi-Fi cannot carry it,
  audio is delayed too, and that still lowers the REMB (tested below).
* **AVCDEC decodes more bits per frame** at the same resolution and frame rate.
  Whether larger frames take longer than HA06-22's 10.5 ms per AU is unknown
  until hardware runs it.
* **Scenes heavier than the Xbox's pacing rate still queue** on the Xbox while
  they last (at most the ~1.3 s plateau), then catch up.
* **During catch-up a picture can be up to 2.5 s past its AU's completion.**
  That is shown rather than frozen, by design.
* **With no audio for 1 s, no delay lowers the REMB.** The Xbox sends audio
  every 20 ms even in silence. Its own TWCC-based estimator still sees our
  transport feedback either way.

## Model results (tests/rtp-order/src/sender_backlog.rs)

The model runs the production reorder, assembly, live edge, request policy and
(new) receiver bitrate request against an Xbox modeled from the captures.

Xbox side:
* encoder sized by scene content alone;
* pacer at REMB x 1.02, at least 0.93 Mbps. That is calibrated at 2000k
  (HA04-20) and 500k (HA06-22) only; 3 Mbps is extrapolated;
* libwebrtc's 2 s queue-time limit.

Vita side: AVCDEC 10.5 ms per AU, a 3-picture output lag, and the worker queue.

| Case | HA06 | HA07 |
| --- | --- | --- |
| HA06-22-like gameplay: 1.4 Mbps, with heavy scenes at 2.8 Mbps (8-18 s) and 3.4 Mbps (40-46 s); closed loop | REMB 2000k -> 500k within 3 s of the first heavy scene. Video median 1.38-1.49 s behind for the rest of the run | No REMB cuts. Median 76-83 ms (the model's floor) through moderate and 2.8 Mbps scenes. The 3.4 Mbps scene is up to 0.75 s behind and current again 2 s after it ends. Longest gap 25 ms, 0 requests |
| Drain after the 3.4 Mbps scene (HA07 request, local limits only) | 1 AU deadline, 9 expired pictures, a 272 ms freeze, 1 incident and a keyframe request | Nothing expired. Longest gap 25 ms, 0 requests |
| Path queue growing 50 ms per second (audio delayed too) | — | REMB still cut 4 times, to 500k |
| HA04-20 game launch (measured pacer) | — | Longest gap 26 ms, 0 requests; up to 1.1 s behind, then current. The local queue peaks at 35 AUs, which would overflow 32 |
| HA05-21 scene change / sender pause / floor keyframe / slower path | — | 289 ms gap / held for the 3.25 s pause / one incident, recovered once / no incident |

## Verification (host only)

`tools/run_host_audit.py --clippy` passes all 30 steps: 411 Rust tests (the
HA06 gate had 365), with strict Clippy on every harness. The new feedback unit
tests also run in rtc-transport, and rtp-order now compiles feedback.rs and
congestion.rs for the sender model, which adds their 30 unit tests there.
`cargo fmt --all -- --check` is clean.

Mutation checks. Reverting each change fails at least one test:

| Reverted | Tests that fail |
| --- | --- |
| Shared-path rule (all video delay counts again) | 8 (feedback unit tests, run standalone and in rtp-order, plus 2 model tests) |
| Ceiling 3 -> 2 Mbps | 4 (2 model tests, the renegotiated SDP, the audit contract) |
| Catch-up limit back to 240 ms | 7 (3 decoder-pump, 4 model) |
| AU queue back to 32 | 1 (HA04-20 model: queue overflow, incident) |
| Draw without the 240 ms stall limit from decode | 4 (decoder-pump) |
| Selection without it | 2 (decoder-pump) |
| Audio evidence that never goes stale | 1 (clock) |

Deliberately changed existing assertions:
* **decoder-pump.**
  * The stale-compressed-work case is now 3 s old (beyond 2.5 s), not 2 s.
  * The 20-minute schedule adds outputs decoded 330 ms after their AUs
    completed: 1,980 of 23,880 selections. Its counts are unchanged.
* **rtc-transport.** The chat renegotiation offer expects `b=AS:3000` and
  `b=TIAS:3000000`. The arrival test passes its injected delay as shared-path
  delay.
* **Feedback unit tests** pass an audio delay; the summary includes `path:`.
* **rtp-order sender model.** The Vita side now models AVCDEC time and the
  worker queue, and the existing cases run with HA07 limits. The HA05-21 scene
  change gap is 289 ms (345 ms in the HA06 model). The other cases keep their
  assertions.

## Remaining limits

* No physical Vita has run HA07, and Cloud is uncaptured.
* The Xbox's pacing at 3 Mbps is extrapolated. If it paces lower, heavy scenes
  lag more.
* Scenes heavier than the pacing rate still lag while they last.
* After a real congestion cut, the REMB still recovers at +100 kbps per 10 s
  (unchanged). From 500k back to 3000k takes about 4 minutes.
* AVCDEC's 3-picture output lag (about 50 ms at 60 fps) and its decode time
  are unchanged.
* The native Vita check, clippy and release build run only in CI (local
  network policy blocks the SDK image).
