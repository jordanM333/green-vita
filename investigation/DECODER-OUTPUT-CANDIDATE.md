# Decoder output servicing candidate — hardware verification pending

The newly supplied per-frame trace changes the diagnosis from aggregate residence
growth to directly matched input/output evidence. This is a candidate correction,
not a claim that the reported gameplay latency is fixed.

## What the supplied trace establishes

The two pasted CSV blocks are separate observations. The first contains only
`present_return` with no picture identity over approximately 10 seconds. It cannot
measure stream freshness. Its short present calls do not establish responsive
video. Do not concatenate its clock with the second block.

The second block contains returned AVC picture PTS. Selected **verbatim rows** are
saved in [test34-pts-selected.csv](test34-pts-selected.csv), with explicit gaps;
this is not the full pasted trace and is not suitable for rate/count estimates.
[Computed associations](test34-pts-selected-analysis.json) use the returned PTS,
not the current call's input timestamp. The user associates the evidence with the
last stream; the CSV itself does not embed a build SHA. The accompanying history
identifies RX Test 34, d7397bf4ccbcce08b7df9069b083be1f15306471.

| Observation | Local trace evidence | Implication |
|---|---|---|
| RTP 1252175405 | Submitted at 54.303690 s, returned at 55.300406 s: **996.716 ms** | A previously submitted picture comes back almost a second later. |
| RTP 1252235525 | `no_picture` after submission at 55.390884 s; returned at 56.821145 s | No output did not mean the input was discarded. |
| Recovery beginning 55.429740 s | Queue-full reason 3, three queued generation drops, then IDR-wait reason 5 | A burst triggers reference recovery and stops new dependent input. |
| Recovery IDR 1252348025 | Submitted at 56.123998 s; returned at 56.906713 s | Admission is not completed visual recovery: another **782.715 ms** elapses inside the decoder path. |
| Recovery IDR 1252488065 | Submitted at 57.826406 s; returned at 58.393606 s | Another **567.200 ms** follows IDR admission. |
| RTP 1252712075 | Submitted at 59.518684 s; returned at 60.139720 s: **621.036 ms** | Substantial decoder residence persists after arrivals become regular. |
| Last returned picture 1252838165 | Submitted at 60.928710 s, returned at 61.540209 s: **611.499 ms**; current decode call 2.838 ms | A fast hardware call can still return an old picture. |

All six selected `no_picture` submissions later return their own PTS. Successive
new-input calls retire older pictures; after recovery many carry the old epoch
and are rejected. The first recovery above takes **1476.973 ms** from recovery
beginning to the first new-epoch output, including the IDR wait. This explains a
substantial measured video-delay/freeze mechanism, not the entire reported
5–6-second button-to-screen response. Network burstiness and admission recovery
remain visible and are not claimed solved.

At the end, input/output RTP separation is about 55,530 ticks, roughly 617 ms of
90 kHz source time (about 37 frames at 60 fps). That is distinct from the matched
local residence. Pending metadata alone is not firmware occupancy, and the
excerpt does not establish an exact whole-session input/output conservation count.

## Correction and API evidence

The old worker calls `sceAvcdecDecode` once per incoming access unit with space
for one output, then waits for another input. A retained input without output can
therefore add persistent backlog unless some separate operation removes output.
Changing the generation only rejects pictures; it does not remove that backlog.

The candidate services output independently with a null pointer and zero-length
ES buffer, without submitting another AU. It alternates one such call with queued
input, drains during input gaps/recovery until no picture is returned, checks stop
between calls, and releases the texture mutex after each call. If every admitted
PTS has returned, it skips the hardware poll entirely; metadata presence permits
a probe but is not taken as proof of firmware readiness. Returned PTS still
controls timing, epoch rejection and feedback. Old-epoch output counts as drain
progress, even though it is never published. No arbitrary compressed P-frame
skipping, DecodeStop, latency-triggered reset, or reconnect is introduced.

Primary implementation precedent: [Vita FFmpeg in wiliwili, pinned source](https://github.com/xfangfang/wiliwili/blob/88e5876bea9502d06f46a8656e3530684d3aaf7d/scripts/psv/ffmpeg/ffmpeg.patch),
`vita_decode_video` uses `au.es.pBuf = NULL; au.es.size = 0; sceAvcdecDecode(...)`
when the input buffer is full. Its end-of-stream `sceAvcdecDecodeStop` path is
separate. This supports using the public output-only operation; it does **not**
prove this stream's pictures are ready on every empty call. That is the remaining
hardware question. If polling returns an error, it is disabled once for the
session, logged and counted; it does not trigger a recurring decoder reset. Such
a run fails the candidate's verification. Existing actual input-decode-error
recovery is retained.

The decoder remains 1280×720 capacity, one reference frame, RGB565 960×544 output.
No changes to authentication, modes, audio, mappings, input admission, REMB, RTP
reordering, queue size or age limit are part of this correction. Their device
behavior still needs acceptance checks. Diagnostics add separate poll counters
and output-only PTS events; they never pretend an empty poll admitted an input.
The 1 Hz history retains 2100 samples (at least 35 minutes), approximately 4 MiB
at the current status size; the detailed event ring remains bounded at 4096.

## Regression evidence and limits

`tests/decoder-pump` compiles the production decoder adapter, worker, texture
ownership, timing, policy and notification code. Its build script only redirects
module paths; it does not substitute those implementations. The SDK functions are
controlled host fakes, not a Vita emulator or a performance benchmark.

- Before: withholding 37 outputs in the old one-call-per-input schedule retains
  37 pictures even after subsequent successful calls.
- After: output-only calls retire those 37 pictures with zero extra submissions
  and zero resets in the same controlled decoder.
- The real threaded worker publishes an initially withheld picture without a
  subsequent input, and stops polling when empty.
- Recovery rejects old-epoch pictures and reaches the new IDR without waiting for
  further input. Published pixels and feedback identity match that IDR.
- Repeated withheld outputs do not accumulate; the displayed texture retains its
  pixels until the renderer takes a replacement.
- An unsupported poll produces one visible failure, not repeated polling/reset
  or duplicate input.

See [host results](decoder-pump-host-tests.txt) and
[analysis tests](decoder-pump-analysis-tests.txt). CI also runs the existing
feedback, SCTP/WebRTC, receiver-report, RTP/H264, metadata and notification gates,
then builds a VPK against the real Vita SDK. The build workflow produces an
artifact only; it does not publish a release or change master. The SDK container
uses the repository's existing `latest` tag; the CI log records its resolved image.

## Device verification and rollback

Install the candidate once and confirm its full embedded revision. First inspect
normal playback in both modes: `Decoder poll calls/pictures/errors` must show
successful servicing with zero errors, and returned-PTS residence must stop
forming the previous 0.5–1.3-second plateau. Poll pictures alone are not success:
use matched decoder residence, receive-to-GPU age, visible response and freezes.

If that fails, do not run the full acceptance matrix or repeatedly refresh to
clear it. Stop once to save the automatic trace/status/history and use RX Test 34
as rollback. Its VPK SHA-256 is
`8ee5ece964b351a25ff264ac6ac467ff23113b5270fc9fffca2772ff04557ae1`.
It uses the same test title ID; do not delete saved authentication/data to roll back.

If the focused check succeeds, the agreed acceptance remains two uninterrupted
30-minute runs per mode, repeated early/late button-response measurements on a
comparable clock, at most 100 ms added drift with uncertainty accounted for,
and no recovery/reset cycle used as a cure. Record absolute response as well as
drift: a stable but already sluggish stream fails. Capture early and late events
and preserve each run's automatic files after stopping (current + previous are
retained; older sessions rotate). Test idle and deliberate network interruption
separately, then check authentication, audio, picture quality, controls and debug.
No on-device acceptance result has been asserted here.
