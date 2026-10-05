# HA09: fixed bitrate request, audio that cannot be muted by timestamp slips, no video banners

The user's report with HA08-24: "It ran well until I started playing a lot
more - it started lagging and never fully recovered. … There is also no audio
now. Remove the banners that happen when video is delayed."

HA09 is a hypothesis until a physical Vita confirms it. Cloud sessions have
still not been captured with the latency folder. Rollback source is
b898ee9659f8d4e214998c7f0b4bc722852d8d45 (HA08-24).

## What HA08-24 showed (Home; latency capture 0-182.6 s, display capture 0-10 s)

* **The capture ended before the lag.** It stops at 180 s and recorded no
  latency trigger ("NOT REPRODUCED DURING CAPTURE"). Up to 182 s, video was
  current: 59-61 fps, decoder-to-GPU about 65 ms, REMB 3000k with no cuts. The
  lag the user saw came later.
* **The Xbox's audio timeline had slipped 316 ms.**
  * Its audio sender reports cover 176.2 s of its own clock but only 175.9 s
    of audio RTP time. Video's agree to 0.0 ms.
  * Audio packets that arrived 8 ms ahead of their RTP time at the start
    arrived 312 ms behind it by 170-182 s (status "Ingress A: rel+320ms").
  * The path was not slow: video was current, and audio packets were full
    (median 358 bytes) and arrived 50 per second.
* **Display capture.** 605 AUs, 2.26 MB. Its 2 lock skips mean it is not a
  complete replay. FFmpeg decodes all 605 without error. In all 37 samples the
  decoded picture equals the uploaded texture (0 mismatches), and every sample
  shows picture content (at least 540 of 576 cells non-black).

## Root causes

1. **No audio.** Each audio packet got a deadline of "expected by RTP time +
   480 ms". The clock's baseline only ever moves earlier. After a slip, every
   later packet looks late by the slip, and once the slip passes about 400 ms
   (480 ms less the output queue), every packet misses its deadline. Then all
   audio is discarded for the rest of the session. The slip was 316 ms by 151 s
   and kept growing under heavier play. This deadline predates HA07.
2. **Lag that never recovered.** HA07 lowered the REMB on "shared" delay, the
   lower of video's and audio's added delay. Audio's included the slip, so it
   read about 320 ms. Once a heavier scene put video further behind than that,
   the shared delay jumped to 320 ms and the request was cut. It could recover
   only after 10 s at 240 ms or less, which the slip prevented. The Xbox then
   paced below the game's output and video stayed behind: HA06-22's failure,
   reintroduced through audio. The host model reproduces this: HA07's rule with
   a 320 ms slip cuts the request to 1470k and ends 0.7 s behind.

## Changes

| Change | Where |
| --- | --- |
| The REMB is fixed at VIDEO_CEILING_BPS (3 Mbps). The receiver no longer lowers it on any measured delay. Delay is still reported in the status line. TWCC and RTCP receiver reports continue, so congestion control is the Xbox's own. | feedback.rs, session.rs |
| The retired growth-based budget (congestion.rs) moves to the rtp-order sender model, which uses it to reproduce the HA06 and HA07 requests. HA07's audio-delay helper is removed. | tests/rtp-order/src/receive_budget.rs, clock.rs |
| Audio has no RTP media deadline. Its local age (240 ms from receipt to playback) and the output queue trims still bound its latency. The audio MediaClock and `MediaClock::deadline` are removed. | rtp.rs, media.rs, session.rs, live_edge.rs, audio_timing.rs |
| No banner over the game: the top banner for interrupted, recovering, waiting or delayed video and the "Reconnecting video…" indicator are removed. The last good picture stays on screen until a current one replaces it. The lag-help state is removed. Help after a manual Refresh and for no picture at all after 30 s remain. | streaming.rs, startup.rs, stream_session/session.rs |
| Build identity HA09. | diagnostic.rs, final-build.yml, build_provenance.py |

## Call-outs: constrained settings that HA09 changes

* **Receive budget:** removed from production; the REMB is fixed at the ceiling.
* **Audio policy:** the RTP media deadline is removed. The local 240 ms age
  limit and the queue trims are unchanged.

Unchanged: the 3 Mbps SDP ceiling and the `maxBitrateKbps` 2000 hint,
resolution, thread priorities, the HA07 local catch-up limits, keyframe policy,
the DISPLAY01 and latency captures, and Home sign-in handling.

## Model results (tests/rtp-order/src/sender_backlog.rs)

| Case | Result |
| --- | --- |
| HA06-22-like gameplay, HA06 request | cut to 500k; video median 1.38-1.49 s behind |
| Same, HA07 request with a 320 ms audio slip (HA08-24) | cut to 1470k; ends 0.7 s behind, median over 1 s for the last 10 s |
| Same, HA09 fixed request, with or without the slip | no cuts; current except during a 3.4 Mbps scene (up to 0.75 s), current again 2 s after it |

The model has no congestion control of its own on the Xbox side. It cannot
show HA09 on a congested path; that case rests on the Xbox's own controller.

## Verification (host only)

* New tests:
  * the REMB stays at the ceiling through growth, a sender plateau and a fixed
    offset (feedback, and over real SRTCP in rtc-transport);
  * a 1 s audio timeline slip never discards current audio (production
    AudioRtp);
  * the HA08-24 slip model under HA07 and HA09;
  * no pixels are drawn over a held picture, nor over the top half of the
    picture for any video state (presentation).
* A host Clippy of the main crate, with a fake SDK, gives exactly the earlier
  baseline warnings, so no new dead code.
* `tools/run_host_audit.py --clippy` on Rust 1.98.1 (the CI toolchain)
  passes all 30 steps: 366 Rust tests (HA08: 412), with strict Clippy on every
  harness. `cargo fmt --all -- --check` is clean. The count fell because the
  removed budget, audio-delay and lag-help tests no longer run, and feedback.rs
  no longer carries the budget's tests into three harnesses.

## Remaining limits

* No physical Vita has run HA09. The lag event itself was not captured.
* A real Wi-Fi capacity drop is left to the Xbox's congestion control. No
  capture so far shows path congestion; every captured lag was inside the Xbox.
* Scenes heavier than about 3 Mbps still lag while they last, then catch up.
