# HA05: no keyframe requests into a sender backlog; recovery keyframes may catch up

This candidate is a hypothesis fix for the HA04-20 game-launch freeze. It is
NOT confirmed until a physical Vita shows video recovering after a sender
backlog in both Home and Cloud sessions. Only one Home session (HA04-20) has
been captured with this pattern. Cloud sessions have not yet been captured
showing it. Rollback source is baca0e5aa34d8382d3bee0c07615bfbb6867d3a6
(HA04-20).

Scope as approved: (1) hold keyframe requests while video is backed up at
the sender, and (b) a bounded catch-up allowance after a recovery keyframe.
One departure from the proposal is called out below: the hold lasts until
video is less than 120 ms late, not 240 ms.

## What the HA04-20 capture shows (re-derived from history.txt and events.csv)

* The game launched at about 114.6 s. Xbox encoder frames grew from about
  3 KB to 5-7 KB (about 2.4-3.5 Mbps). The Xbox delivered about 2.1 Mbps.
* Only video was delayed: audio stayed about 5 ms late and ICE RTT stayed at
  6-17 ms. Audio and video share one BUNDLE flow, so the queue was on the Xbox
  side, not the network or the Vita. Video lateness peaked at 1.81 s at
  120.3 s.
* The incident (quarantine) was at 115.42 s. HA04 requested keyframes at
  115.42, 115.72 and 116.32 s, then about every 1.0 s until 125.34 s: 12 in
  all. The Xbox answered every request with a two-frame keyframe of 37-48 KB.
  The keyframes' RTP spacing (300, 601, then 1001 ms) equals the request
  spacing. Each arrived 0.64-1.67 s after its request, behind the backlog,
  and was rejected: 1727 `stale_rtp_rejected` after 112 s.
* Those keyframes account for about 0.7-0.9 s of the 1.8 s peak.
* Our REMB stepped down 2000 -> 1400 -> 980 -> 686 -> 500 kbps
  (`receiver_ceiling_bps` at 116.39-119.74 s). From about 119.5 s, P frames
  were about 1 KB (about 0.5 Mbps) and delivery was about 0.94 Mbps. Each
  keyframe pair then took 300-353 ms on the wire every second. The backlog
  shrank about 0.5 s per second between keyframes, but only about 0.1 s per
  second overall (1.63 s at 120.0 s, 1.0 s at 125.4 s). The capture ended at
  125.8 s, still frozen, with video 1.0 s behind.
* The last good frame was held throughout (`video_hold_begin` at 115.424 s,
  HA04 D). That is the freeze the user saw while audio continued.
* Recovery was already marginal at 2 Mbps. At 113.3 s (incident 2), the IDR
  picture expired at 240.5 ms local age (`picture_expired` a=240490). AVCDEC
  releases a picture only after three newer AUs (`decoder_output_pts` lags
  `decode_submit` by 3 AUs), so the following picture recovered instead. At
  about 0.95 Mbps a keyframe pair takes about 350 ms. The frames behind it
  then exceed the 240 ms recovery budget and re-quarantine, so recovery
  cannot succeed until REMB climbs back (+100 kbps per 10 s).
* The uploaded pipeline-status.txt and pipeline-incidents.csv are from build
  AR01-4 (68dc5032), not from this run.

## Changes

| Classification | Change | Where |
| --- | --- | --- |
| HA04 REGRESSION (keyframes into a backlog) | Recovery requests wait while video keeps arriving at least 120 ms late: `keyframe_request_suppressed` reason 4. They go out as soon as video is under that line. Silence (no new media) and current-but-undecodable video keep HA04's 300 ms -> 600 ms -> 1 s backoff unchanged. | live_edge.rs `backlogged`, `pending_request` |
| SAFEGUARD (slower path) | Lateness that stays between 120 and 240 ms for 1 s is a slower path, not a draining backlog: the backoff resumes, because such a keyframe is admissible. | live_edge.rs `late_since` |
| RECEIVE BUDGET (approved option b) | For 1 s after a recovery IDR is admitted, frames up to the existing 480 ms ceiling are accepted and do not count as incident pressure. Then the usual 240 ms rule with its 120 ms confirmation applies again (`live_edge_catch_up_end_ms`). | live_edge.rs `CATCH_UP`, `evaluate`, `ingress_useful` |
| RECEIVE BUDGET (part of b) | During recovery a frame is judged by when its first packet arrived, so a keyframe's own transmission time does not count as lateness. | live_edge.rs `arrival_delay` |
| TESTABILITY (no Vita change) | AU admission uses `max(Instant::now(), received_at)`. On the Vita now is always later than the completing packet's receive time, so this is identical there. Host replays drive time through `received_at`. | rtp.rs |
| DIAGNOSTICS | `live_edge_catch_up_end_ms` (a = lateness when the allowance ended). The status line adds `catch-ups ok/late:N/M`. | worker.rs, trace.rs, DIAGNOSTIC-SCHEMA.txt |

Why 120 ms, not the proposed 240 ms. The 120 ms line is half the ingress
budget, the clock's existing healthy-path bound. In the model below, asking
as soon as video is within 240 ms lands the keyframe behind about 220 ms of
queue at the bitrate floor. The frames behind it then exceed 480 ms and
recovery loops: 18 requests, 9 incidents, never steady. 180, 120 and 60 ms
all recover. 120 ms leaves about 60 ms of margin under the 480 ms ceiling.

Unchanged: bitrate and REMB policy (floor 500 kbps, +100 kbps per 10 s),
resolution, SDP ceiling, thread priorities, audio policy, the 240 ms
presentation local-age limit, the 480 ms ceiling, the 120 ms confirmation,
the 60 ms repair grace, `keyframe_cutover_units`, and the decoder, texture and
GXM path. The clock is never rebased. A standing offset over 240 ms still
blocks video by design, as before, now without futile requests.

## Model: the HA04-20 backlog in the host gate

tests/rtp-order/src/sender_backlog.rs runs the production reorder, H.264
assembly, live edge and request decision. It models only the Xbox side and the
decoder output, calibrated to HA04-20. In the model:

* Frames are 3 packets, 5 after the launch, and 1 after the encoder backs off.
* A FIFO pacer sends 232 packets/s, and 104/s at the floor.
* Each request is answered by a 21+21 packet keyframe (20+16 at the floor).
* AVCDEC releases a picture after three newer AUs.
* A picture older than 240 ms expires.

The back-off (5.5 s) and floor (6.7 s) times are fixed from the capture, not
driven by our REMB.

| Same model | HA04 (baca0e5) | HA05 |
| --- | --- | --- |
| Keyframe requests after the incident | 18 (every 1 s) | 1 (once drained) |
| Sender backlog peak | 2174 ms | 1038 ms |
| Recoveries | 0 | 1 |
| Video resumed | never (frozen 16.8 s to the end) | 4.6 s after the freeze began |
| Worst shown age after resuming | n/a | 418 ms (catch-up) |

## Verification (host only)

`tools/run_host_audit.py --clippy` with Rust 1.98.1: all 30 steps pass, 363
Rust tests (the HA04 gate had 358: +4 rtp-order, +1 decoder-pump, which also
compiles live_edge_tests.rs), with strict Clippy on every harness. `cargo fmt
--all -- --check` is clean.

New tests:

* `game_launch_backlog_gets_no_keyframe_requests_until_it_drains_then_recovers`:
  nothing is requested into the backlog. One request after it drains gives
  one incident, one recovery and a completed catch-up.
* `a_keyframe_slower_than_the_budget_recovers_at_the_bitrate_floor`: a
  keyframe pair at 104 packets/s shows frames over 240 ms late and still
  recovers once, without a loop.
* `a_steadily_slower_path_is_not_mistaken_for_a_backlog`: the path slows
  while video arrives, then settles 150 ms slower. Requests resume 1 s after
  the incident and video recovers.
* `a_recovery_keyframe_is_judged_by_its_first_packet_and_may_catch_up_for_one_second`
  (unit): first-packet admission, 400 ms frames accepted inside the window,
  over 480 ms rejected, and a new incident confirmed after the window.

Mutation checks. Each removal fails at least one test:

| Removed | Tests that fail |
| --- | --- |
| The request hold | 5 |
| The catch-up | 5 |
| The slower-path safeguard | 1 |
| First-packet judgement | 2 |
| Threshold set to 240 ms | 4 |

Deliberately changed existing assertions:

* The 30-minute plateau unit test and the 5,003-packet DIAG03-8 replay
  asserted requests through the stale plateau.
  * They now assert none into it. Both are deferred with reason 4, not
    abandoned.
  * The replay sends 2 requests while video is briefly current again
    (114.09 and 114.40 s, under 120 ms late) instead of 12. Every other
    replay assertion is unchanged.
  * In the plateau test, the first request follows at once when current media
    returns.
* `failed_or_expired_keyframe...` now starts its incident by silence, not by a
  2 s late frame, which is now a backlog.
* `post_idr_frames_that_only_follow_pre_idr_loss...` now counts 16 screened
  packets instead of 30. The 14 newer pre-IDR packets fall inside the catch-up
  allowance and are dropped by reorder and assembly as already passed. There
  is still no quarantine, no epoch bump and no re-entry.
* `stale_media_beyond_the_admitted_idr_still_quarantines` now accepts a packet
  400 ms late inside the allowance. A 500 ms late packet still quarantines,
  and requests still continue afterwards.

## Remaining limits

* No physical Vita has run HA05, and Cloud is uncaptured. The model is
  calibrated to one Home capture, and the Xbox encoder and pacer are inferred
  from it.
* The backlog itself remains: in the model video still freezes about 4.5 s at
  a game launch. Shortening that needs a bitrate, SDP-ceiling or
  show-late-video change, which is out of scope and unchanged.
* After a backlog, REMB sits at 500 kbps and climbs 100 kbps per 10 s, so
  video can look soft for up to about 2.5 minutes (unchanged policy).
* For up to 1 s after a recovery, shown pictures may be up to about 480 ms
  behind (418 ms worst in the model) while the keyframe clears.
* The IDR picture itself still expires at the 240 ms local-age limit when it
  arrives slowly. Recovery then comes from the next picture, as in HA04-20
  incident 2.
* The 120 ms line and the 1 s safeguard are local choices from the model, not
  documented Xbox behavior.
* The native Vita check, clippy and release build could not run locally
  (network policy). CI's native jobs are the authority.
