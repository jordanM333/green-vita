# Streaming V2 test baseline

This branch replaces the experimental FPS-dependent compressed-video queue and
damage-score recovery policy. It is not a new Xbox client or a rewrite of the
security/WebRTC stack. Authentication, Home streaming, input mapping, audio,
decoder allocation, output format and the working texture ownership are reused.

## Invariants

- TitleID GRNVTEST1; isolated data under ux0:data/green-vita-540-test.
- Request 1280x720, 30fps, 2000Kbps; hardware capacity 1280x720; output 960x544.
- Known-incomplete AUs never enter AVCDEC. Following damage, dependent frames
  wait for a complete IDR that actually enters the queue. No FPS damage heuristic.
- Decode queue has three slots and a 50ms age budget, checked before decoding.
  This is not an end-to-end guarantee; scheduling can delay deadline checks.
  Assembly age is measured separately: a large IDR is not rejected merely
  because receiving it took longer than the decode-queue budget.
- Compressed queue overflow/staleness and unavailable output invalidate queued
  reference work and signal recovery. They are not silent frame replacements.
- Decoder resets occur on hardware failures, not ordinary packet loss. Decoded
  pending pictures may be replaced without damaging H.264 reference state.
- Existing 6ms/24-packet reorder grace remains; it is not a playout buffer.
- Pending H.264 assembly is capped before completion (2MiB/2048 packets), not
  just after a marker arrives. Ordered fragments no longer trigger per-packet
  scans/sorts of the growing packet list.

## Diagnostics

The pause-menu diagnostics toggle is retained. The overlay is compact; full
status and a bounded 4096-event metadata ring are saved when leaving streaming:

- ux0:data/green-vita-540-test/pipeline-trace.csv
- ux0:data/green-vita-540-test/pipeline-status.txt

Each new saved session replaces these two diagnostic files only. No payloads,
tokens, account identifiers or URLs are recorded. No filesystem writes happen
in the live receive/decode loop. If the app is killed, the trace is not saved.

Trace stages include first AU packet, complete/abandoned AU, drop reason,
keyframe request, decode submission/return, produced picture generation,
texture acquisition and SDL present return. Drop reason values: 0 sequence,
1 FU-A, 2 malformed, 3 queue, 4 SPS, 5 IDR wait, 6 other. Generation values are
identifiers, not microseconds. `present_return` is not physical display scanout.
Submission RTP timestamps are not guaranteed output PTS: AVCDEC can produce
no picture, so do not assume every output belongs to the immediately prior AU.

Sender reports are captured before the inner interceptor consumes them. Absolute
sender age remains conditional on valid epoch NTP and synchronized clocks;
relative arrival growth is not absolute capture or controller-to-display delay.

## What remains unproven

The server's actual bitrate/FPS can differ from the request. This baseline does
not introduce a new bandwidth controller or claim the request is an enforced
cap. Sustained loss can still cause IDR-wait freezes: this is preferable to
silently decoding damaged references, but is not itself a solution to network
loss. Host regression tests and a successful VPK build cannot establish Vita
gameplay latency; both Cloud and Home require on-device validation.
