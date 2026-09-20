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
  Process each already-delivered receive batch before expiring sequence gaps.
  Capacity pressure still releases packets immediately at the same fixed bound.
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
Saving is attached to RTC session teardown, including a remote Closed event or
worker error. The initial V2 build saved only from explicit close(), which a
remote-first shutdown could bypass. Use the in-app pause menu's Exit game,
wait for the game/console list, then refresh the FTP directory listing.

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

## Receive-path follow-up from the supplied trace

The September 20 trace demonstrates recovery starvation, not a seconds-long
compressed decode queue. At 61.241415 seconds an AU is abandoned; complete AUs
then receive drop reason 5 until the next decode submission at 65.099105 seconds:
a 3.857690-second gap. Earlier gaps are about 0.469, 0.763, 3.237 and 2.176 seconds.
The decoder takes about 3ms when fed. The status reports 1,803 IDR-wait drops,
528 sequence drops, 358 FU drops, and one queue-full drop. These are event
counts with different populations; `assembled` excludes abandoned AUs, so
`drop / assembled` is not a valid frame-loss percentage.

The 4,929 reorder `lost` positions are packets absent at the local deadline,
not proven Wi-Fi losses. 767 packets arrive after the receiver has advanced.
The receiver previously expired a hole before inserting the very packet that
could fill it, and could also expire between packets in one received batch.
The host regression feeds a delayed batch containing an overtaking next AU and
the missing FU-A tail; all three exact AUs must decode with no resync. Tests
also retain genuine-loss, capacity, wrap, and incomplete-FU rejection coverage.

Video payload is measured at 7,291kbps despite a 2,000kbps capability request.
Restore a 2,000,000bps REMB ceiling only when the accepted video payload in the
answer negotiated goog-remb. Receiver reports remain enabled and registered.
Send at most once per 500ms, including failed attempts; no authentication,
encryption, dimensions, FPS, audio, decoder or input changes. This is a fixed
maximum matching the request, not a new bandwidth estimator. `queued` means
accepted for local RTCP transmission, not acknowledged by Xbox. Compare
measured `Video payload` with `REMB`; after four successful queue operations,
`over-windows` counts reporting windows above 2,500kbps (25% burst allowance).
The earlier REMB experiment lacked working receiver reports, so it did not
test this combination. Server compliance and playback improvement remain
on-device questions, not compilation claims.

Retain rejection of known-incomplete AUs and IDR recovery for damaged reference
state. Removing that gate alone could restore motion by reintroducing reference
corruption. The fixes target avoidable damage and excessive incoming traffic.
Recovery current/maximum wait is now visible and traced at entry and exit.
Source NTP offset is labelled `SR offset*`, with relative arrival-delay growth
shown even when sender reports exist. The nearly equal 3.19s audio/video offset
does not prove 3.19s of buffering: the two device clocks were not calibrated.

Build name: GreenVita RX Test; TitleID remains GRNVTEST1. Home mode, rear-touch
swap and the pause-menu diagnostics toggle are preserved. Earlier session-exit
save fix is included. No stock app files are touched.

## What remains unproven

The server's actual bitrate/FPS can differ from the request and feedback. The
trace does not locate missing packets at the access point, kernel, transport,
or application boundary. Sustained real loss can still cause IDR-wait freezes;
recovery waiting is not itself a solution to loss. Host regression tests and a
successful VPK build cannot establish Vita gameplay latency. No new NACK path
is enabled: retransmissions with the observed 183ms RTT would arrive after the
6ms assembly deadline and require a separately designed recovery strategy.
