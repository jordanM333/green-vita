# HA06: continuous playback — late video plays and catches up instead of freezing

This candidate is a hypothesis fix for the freezes in HA04-20 and HA05-21.
It is NOT confirmed until a physical Vita shows it in both Home and Cloud
sessions. HA05-21's status reports Mode:Home even though the DISPLAY01 README
asks for Cloud, so Cloud remains uncaptured. Rollback source is
bb0c2ab2726171adc0edb87fbb9acea2386162ca (HA05-21).

Direction from the user after HA05-21: "Can we implement the fix that is going
to make it perform as I want? We've gathered enough data. Fix the issue."
HA06 therefore changes the live-edge contract deliberately.
LIVE-EDGE-REMEDIATION.md called its 480 ms ceiling "a conservative safety
ceiling... The final hardware UX budget remains to be validated". The hardware
results since then are freezes, and those freezes come from that policy.

## What HA05-21 showed (Home session, latency trigger 21.6 s, DISPLAY01 first 10 s)

* **Freeze 1, 9.92-13.17 s.** The Xbox stopped sending video mid-frame for
  3.25 s.
  * Frame 1724008184's second packet (seq 15725) arrived 3.25 s after its first
    (seq 15724).
  * Sequence numbers were continuous and audio continued at 50 packets/s.
  * The Xbox resumed with an IDR.
  * HA05 held the last frame and requested keyframes at 10.28, 10.58, 11.18 and
    12.18 s, all unanswered. It recovered 130 ms after the IDR arrived (13.302 s).
  * The client cannot show video the sender did not send.
* **Freeze 2, 20.71-22.28 s (1.6 s).**
  * A 78.8 KB (66-packet) scene-change frame took 310 ms to arrive at about
    2.1 Mbps.
  * The decoder worker's 240 ms local deadline counted from the AU's first
    packet. So it expired the frame (`au_deadline_recovery`), broke the
    reference chain and forced a keyframe wait.
  * Frames behind it arrived 0.2-0.48 s late. The live edge treated that as an
    incident too (21.15 s).
* **DISPLAY01 shows the display path is not involved.**
  * In all 26 samples, decoder output, uploaded texture and OS framebuffer agree
    at every sampled cell. There were zero upload mismatches, and 520-573 of 576
    cells were non-black.
  * FFmpeg decodes the complete submitted.h264 (427 AUs) without error.
* **Between incidents, playback was healthy:** 55-64 fps decoded and shown,
  ingress lateness 0-10 ms, and receive-to-GPU 70-90 ms.

## Root cause (HA04-20 and HA05-21)

Apart from the sender's pause, the freezes were the client's own policy:

1. **The 240 ms local budget measured transmission time as local delay.** It
   counted a frame's own transmission (age since its first packet) as local
   queueing, so large frames were discarded.
2. **The live edge turned every sender backlog into a freeze plus a keyframe
   wait.** Its budgets were 240 ms of ingress lateness, or 480 ms at once. A
   keyframe requested into a backlog queues behind it. So recovery could only
   follow the drain, and each request lengthened the backlog (HA04-20: 12
   keyframes of 37-48 KB).

## Changes

| Change | Where |
| --- | --- |
| The local budget starts when an AU completes. The 240 ms checks now measure only Vita-side queueing: the decoder deadline, picture and presentation expiry, and redraw. | rtp.rs (submit with completion time), worker.rs |
| Late video plays. Below `LAG_CEILING` (2.5 s) lateness is not an incident: video is admitted, decoded and presented, and the newest picture is always shown, so it catches up as the sender drains. Only video arriving more than 2.5 s late is an incident. The video presentation deadline is expected + 2.5 s. Audio keeps its 480 ms deadline. | live_edge.rs |
| Silence is not an incident. After 1 s without new video, keyframes are requested on the HA04 backoff (300 ms -> 600 ms -> 1 s, never stopping). If video resumes on the same reference chain, it plays without waiting for a keyframe. | live_edge.rs `SILENCE`, `pending_request` |
| A broken reference chain while live (loss, a failed AU, the decoder deadline) is an incident. Damage recovery therefore uses the HA04 backoff instead of a request every 300 ms, and ends with a presented picture. | live_edge.rs `damage` |
| A request is not repeated sooner than the current video lateness + 300 ms, because the previous keyframe can still be queued behind the sender's backlog. `keyframe_request_suppressed` reason 4 now means "in flight". This replaces HA05's backlog hold. | live_edge.rs `in_flight`, `keyframe_request` |
| Removed, because HA06 supersedes them: HA05's 120 ms backlog hold, its slower-path guard and its 1 s catch-up window. | live_edge.rs |
| Diagnostics: label, schema notes, and `lag-ceiling:2500ms` in the status line. | diagnostic.rs, DIAGNOSTIC-SCHEMA.txt |

## Trade-off (deliberate)

* **Sender backlogs show late video instead of freezing.**
  * Video is shown up to the backlog's lateness: about 1.1 s in the HA04-20
    model, about 0.5 s at HA05-21's scene change.
  * It becomes current by itself when the Xbox catches up.
  * Audio is not delayed to match.
* **Persistent sender lag now plays late instead of freezing.** For example,
  DIAG03-8's 1.6 s plateau. This reverses the remediation's "no stale
  playback" rule for lateness under 2.5 s.
* **Real interruptions still hold the last frame** with "Reconnecting video…".
  These are loss, or the Xbox not sending video.

Unchanged:
* Bitrate and REMB (500 kbps floor, +100 kbps per 10 s), the 2 Mbps SDP
  ceiling, resolution, thread priorities.
* Audio policy and its 480 ms deadline.
* The 240 ms local budget's value, NACK and reorder (60 ms),
  `keyframe_cutover_units`, and SPS/PPS injection.
* The decoder, texture and GXM path, the hold indicator, and the DISPLAY01 and
  latency capture formats.

## Model results (tests/rtp-order/src/sender_backlog.rs)

These run the production reorder, assembly, live edge and request policy
against Xbox behavior modeled from the captures.

| Case | HA04 | HA05 | HA06 |
| --- | --- | --- | --- |
| HA04-20 game launch | frozen to the end (16.8 s+), 18 keyframe requests | 4.6 s freeze, 1 request | longest gap 26 ms, 0 requests; video up to 1.1 s behind, then current |
| HA05-21 scene change | (not modeled) | 1.6 s freeze on hardware | 345 ms gap (the frame's own 0.3 s transmission), 0 requests |
| HA05-21 sender pause | — | — | held 3.25 s; picture back 210 ms after the sender resumes |
| Outage with loss at 500 kbps | — | recovers once | recovers once, 1 request |
| DIAG03-8 replay (real timing) | incident, frozen | incident, frozen | plays up to 1.6 s late, 0 incidents, 0 requests |

## Verification (host only)

`tools/run_host_audit.py --clippy` with Rust 1.98.1 passes all 30 steps: 365
Rust tests (the HA05 gate had 363), with strict Clippy on every harness.
`cargo fmt --all -- --check` is clean.

Mutation checks. Each removal fails at least one test:

| Removed | Tests that fail |
| --- | --- |
| Local age from AU completion | 2 |
| The 2.5 s ceiling (set back to 480 ms) | 7 |
| In-flight spacing | 1 |
| Damage while live as an incident | 6 |
| Silence held without an incident | 2 |

Deliberately changed existing assertions:
* **Live-edge unit tests.**
  * The silent stream is no longer an incident and requests start after 1 s.
  * The 30-minute 1.595 s plateau now plays.
  * A 3 s delay, not 2 s, is beyond the ceiling.
  * The failed-keyframe test starts its incident by damage.
  * The SR-plateau test asserts the lag stays measured.
* **DIAG03-8 replay.** It now asserts zero incidents and zero requests, with
  all video presentable (1604 ms maximum).
* **HA04 regressions.**
  * After the outage the state is Live with silence requests, and the incident
    starts when lost packets break the chain.
  * The recovery IDR is counted as an IDR submission, not a cutover (no
    quarantine sets refresh).
  * 10 pre-IDR packets are screened instead of 16.
  * The stale-quarantine case uses 2.6 s.
* **Repair tests.**
  * The out-of-order AU's submit time is its completion, not its oldest packet.
  * The quarantine case is 3 s late.
* **Decoder-pump.** The held-frame test uses damage instead of silence. The
  stale cases are 3 s late.

## Remaining limits

* No physical Vita has run HA06, and Cloud is uncaptured.
* Sender-side lag still exists; it is shown instead of frozen. Long lag
  episodes like RX38's (1-2 s for tens of seconds) will look laggy. Fixing
  their cause would need bitrate or SDP-ceiling changes, which are unchanged.
* The Xbox pausing video, as during HA05-21's game launch, cannot be fixed
  client-side.
* A large scene-change frame still pauses the picture for its own transmission
  time (about 0.3 s at 2 Mbps).
* Up to three pictures AVCDEC holds meanwhile may expire at output. This is a
  skip, not a recovery.
* The 2.5 s ceiling and the 1 s silence threshold are local choices, not
  documented Xbox behavior.
* The native Vita check, clippy and release build run only in CI (local network
  policy blocks the SDK image).
