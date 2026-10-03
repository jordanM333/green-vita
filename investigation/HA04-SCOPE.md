# HA04: keyframe recovery that does not give up, held last frame

This candidate is a hypothesis fix for the HA03-19 blackout. It is NOT
confirmed until a physical Vita shows video recovering after loss in both Home
and Cloud sessions. Only a Home session has been captured with this pattern;
Cloud sessions have not yet been captured showing it. Rollback source is
ebce1ac0826390fa8d08bd5927742bc6a7d1c15d (HA03-19).

## What the HA03-19 captures proved (as reported, not re-derived here)

* The display pipeline is pixel-correct: DISPLAY01 decoder output, uploaded
  texture and OS framebuffer agree across all 37 samples, and an independent
  FFmpeg decode of submitted.h264 matches the Vita frame for frame. The ~2.3 s
  of black at stream start is server content. Decoder/texture/GXM unchanged.
* Home latency capture (trigger_us 105980277): Wi-Fi loss at 105.5 s, keyframe
  requests at 105.52/105.82/106.42/108.27 s, IDR admitted at 108.52 s
  (recovery_idr_admitted_ms=2998), recovery_begin again at 108.60 s, requests
  at 108.60/109.20 s, then none for 6.8 s with "requests:3/3". About 1000 AUs
  dropped as IDRwait, IDR count stuck at 16, nothing decoded or shown. Audio
  continued. Video froze, then turned black.

## Confirmed in code (ebce1ac)

The code agrees with that reading on every point.

* The cap was `MAX_REQUESTS = 3` (live_edge.rs:16), enforced by
  `LiveEdge::request_due` (live_edge.rs:352-366) with 300 ms then 600 ms gaps.
* Per incident, with exactly one rearm. A new live-edge incident resets the
  count. The single `fresh_request` rearm fires once per incident when current
  media returns (live_edge.rs:260-268): the 108.27 s request.
* `damage()` (AwaitingPicture -> AwaitingKeyframe) never resets it. The
  108.60/109.20 s requests are 2/3 and 3/3 of the same incident (a new incident
  would have produced 108.60/108.90/109.50). After that, nothing could ever ask
  again: while recovering, `request_keyframe` ignored the RTP layer's IDRwait
  demand and deferred only to the exhausted live edge (session.rs:592-619).
  The Xbox sends IDRs only on request, so the stream could not recover.
* The 108.60 s re-entry went through `VideoRtp::record_damage`, the only
  emitter of recovery_begin, which also calls `LiveEdge::damage()`. Within
  80 ms of an admitted IDR that can be reached by:
  1. a late packet older than the IDR (NACK retransmission or AP-buffered
     outage media) failing the 240 ms recovery ingress check
     (`stale_rtp_rejected`, media.rs:191-203/255-263). This quarantined the
     whole chain: queue discarded, epoch bumped, the IDR's picture invalidated;
  2. loss in the first post-IDR P frame (`au_abandon`);
  3. a post-IDR P frame over the 240 ms recovery budget (`stale_au_rejected`);
  4. the decoder thread expiring the IDR or a decode error
     (`au_deadline_recovery`, worker.rs:428-455).
  The summary data cannot distinguish these. The capture's events.csv between
  108.52 and 108.60 s (post section) records whichever fired. Only (1)
  restarts recovery for media that is already irrelevant; HA04 fixes it.
  (2)-(4) genuinely break the new reference chain and remain recoveries, which
  HA04 now always completes by continuing to ask.

## Changes

| Classification | Change | Where |
| --- | --- | --- |
| CONFIRMED DEFECT (terminal stop) | Requests never stop while video waits for a keyframe: 300 ms -> 600 ms -> 1 s, then every 1 s. The backoff resets when an admitted IDR yields a presented picture. Current media returning after a stall restarts the backoff, never closer than the 300 ms cooldown. An admitted IDR gets 1 s to produce a picture before asking again. Every path uses one decision, `LiveEdge::keyframe_request`. | live_edge.rs, session.rs |
| CONFIRMED DEFECT (premature re-entry) | A packet failing the ingress deadline whose timestamp the assembler has already passed is dropped (`stale_late_rtp_ignored`), not quarantined. Stale media at or beyond the current AU still quarantines as before. | rtp.rs `reject_stale`, media.rs |
| ROBUSTNESS (pre-IDR loss) | Losing a whole frame immediately before the IDR made the assembler hold the intact IDR as Pending, then discard it. A sequence gap before an AU is now tolerated only when the AU's first slice is an IDR slice with first_mb_in_slice 0. P pictures and IDRs missing slice 0 are unchanged. | rtp.rs `starts_idr_picture` |
| DESIGN CHANGE (display) | The last good picture is held on screen during recovery, stalls and epoch changes, with a small "Reconnecting video…" label. It blanks only on session end or output replacement. See below. | surface.rs, video/mod.rs `video_held`, streaming.rs |
| CONFIRMED DEFECT (parameter sets) | A new decoder submits nothing before its first IDR (HA03-19 submitted.h264 began with a slice referencing PPS 0). Every IDR that ends a wait gets the cached SPS/PPS it lacks ahead of its first slice. An IDR carrying its own sets stays byte-identical. With nothing cached, startup and live-state waits submit the IDR as before; live-edge recovery and refresh still drop it. | rtp.rs, parameters.rs, policy.rs |

`keyframe_cutover_units` semantics are unchanged. Existing live-edge safety is
unchanged: stale media is rejected at the same budgets, recovery still requires
a current IDR and a presented current picture, and the clock is never rebased.

Keyframe pacing changed (requested). No bitrate, resolution, receive budget
(240/480 ms, 120 ms confirmation, 60 ms repair), thread priority, audio policy
or SDP ceiling changed. The 3-request cap's only documented source was
LIVE-EDGE-REMEDIATION.md step 3 ("not an endless PLI loop"), a local design
choice. No server or protocol limit is documented for videoKeyframeRequested
or PLI. The 1 s ceiling is a local rate limit, never a stop. ClockUncertain
(a clock discontinuity or a gap over 120 s) still does not request: it rejects
all media until restart, and its suppression is now logged.

## Why expire-to-black existed, and why holding is the safe alternative

Black was deliberate (LIVE-EDGE-ISSUES.md row 3, LIVE-EDGE-REMEDIATION.md step
5). C05 displayed decoder output up to 22 s old as if live. "A texture is not
a continuing authorization to display historical video." The hazard was stale
pictures presented as current: advancing old video, false rendered-frame
feedback, false recovery declarations.

HA04 keeps every one of those checks for current video. `can_draw` still
revalidates epoch, local age and live-edge state every draw. Only a picture
that passes is reported as presented, so it alone produces frame feedback and
can declare recovery. Stale decoder output is still rejected at publish and
selection. The hold redraws only the texture last selected as current. It
never advances, never counts as a presentation, and is labelled as
reconnecting. Ownership is unchanged. The texture is the surface's own SDL
copy, and the decoder never writes the displayed slot. Detach still revokes
leases before CDRAM and OwnedTexture release, and SDL allocations after detach
remain [80, 80]. An epoch bump invalidates decoder output, not an uploaded
copy, so only session end or output replacement blanks the screen.

## Diagnostics

DISPLAY01 and latency captures are unchanged in format (schema 1). New stages
are documented in DIAGNOSTIC-SCHEMA.txt (README.txt in each capture):
`keyframe_request_backoff_ms`, `keyframe_request_suppressed` (1 cooldown,
2 admitted-IDR grace, 3 clock uncertain), `stale_late_rtp_ignored`,
`video_hold_begin`/`video_hold_end`. The status line shows `requests:N
backoff:Xms total:T` and `staleLateIgnored:N`. `recovery_idr_admitted_ms` no
longer fires for the initial first-IDR wait. Pixel samples record `drawn` when
the sampled texture reached the canvas, held or current.

## Verification (host only)

`tools/run_host_audit.py --clippy` with Rust 1.98.1: all 30 steps pass, 347
Rust tests (HA03 baseline 337) and strict Clippy on every harness. New tests,
each confirmed to fail when its fix is reverted:

* rtp-order `first_idr_broken_by_further_loss_keeps_requests_going_until_decoding_resumes`:
  production reorder, assembly, live edge and request decision through a 3 s
  outage. The next two requested IDRs each lose a fragment. Nine requests
  precede the intact IDR (pre-HA04 maximum: four), every gap is 300 ms to
  1 s, and the backoff restarts at 300 ms after recovery.
* rtp-order `post_idr_frames_that_only_follow_pre_idr_loss_do_not_reenter_recovery`:
  a frame lost just before the IDR, then 30 stale pre-IDR retransmissions
  after admission. There is no quarantine, no epoch bump and no re-entry,
  and post-IDR pictures recover. `stale_media_beyond_the_admitted_idr_still_quarantines`
  keeps the live-edge contract for stale media the assembler has not passed.
* decoder-pump `production_surface_holds_the_last_good_frame_through_recovery_until_session_end`
  on the real surface: the held pixels survive an incident plus an epoch bump
  over 20 redraws, with no presentation report and no false recovery. A
  current picture then replaces the held one and confirms recovery. Session
  end blanks and releases.
* decoder-pump `every_decode_epoch_gets_sps_and_pps_ahead_of_its_first_slice`:
  real x264 output with a mid-GOP join, separately sent parameter sets and
  a loss-forced recovery epoch. FFmpeg (`-err_detect explode`) reports
  "non-existing PPS" for the pre-HA04 join pattern and decodes the HA04
  submission cleanly and pixel-identically.
* presentation `held_frame_indicator_is_small_bottom_right_and_only_shown_while_held`
  at the production 1.3 scale: about 7100 changed pixels (1.4%), bottom
  right only.

Deliberately changed existing assertions: the three live-edge tests and the
5,003-packet DIAG03-8 replay asserted exactly three requests (now bounded
cadence that never stops). Every other replay number is unchanged:
quarantine at 113.717 s/338 ms, 1604 ms maximum, 2170 rejected, audio
34.9 ms. The replay now logs 12 requests over 8.3 s, minimum gap 315 ms. The
HA03 surface test asserted black after expiry: it now asserts the held frame,
then black after session end, with SDL allocations unchanged at [80, 80]. The
manual-refresh test asserted no keyframe wait before the stream's first IDR:
it now asserts that refresh adds none. The host type-check and Clippy of the
main crate (Vita-only session.rs/media.rs/surface.rs/UI) match the HA03
baseline exactly.

## Remaining limits

* No physical Vita has run HA04. Whether the Xbox answers 1 Hz requests with
  IDRs, rate-limits them, or adds latency or bandwidth spikes is unknown,
  especially for Cloud. Request cadence on hardware must be read from the
  capture.
* The specific 108.60 s trigger is not identified (see list above). If it was
  (2)-(4), HA04 recovers by continuing to request rather than by avoiding the
  re-entry.
* While the live edge is Live, the RTP-level IDR wait still requests at the
  300 ms cooldown without backoff (unchanged).
* A large IDR arriving slowly can still expire at the decoder by first-packet
  age, and pre-IDR reorder holes can add up to 60 ms before admission. Both are
  existing budgets, unchanged.
* Injected SPS/PPS precede a leading AUD (pre-HA04 placement). FFmpeg accepts
  this; AVCDEC acceptance is unverified.
* The held frame can stay visible indefinitely while the session is alive,
  with the label shown. Whether that reads well during long outages is a UX
  judgement for hardware testing.
* tests/presentation's original overlay test renders at 1.69 px/pt (scale
  applied twice), so its bottom-row overlay is off-canvas. The new indicator
  test uses the production 1.3 scale.
* The native Vita check/clippy/release build could not run locally: this
  environment's network policy blocks the SDK image's blob host. The host
  type-check and Clippy of the main crate are identical to the baseline. CI's
  native jobs are the authority. Vendored SCTP endpoint tests (untouched)
  need IPv6 loopback, which this container lacks; CI runs them.
