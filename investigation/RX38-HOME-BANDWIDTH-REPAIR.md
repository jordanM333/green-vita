# Home delay and session-preserving video repair

Candidate source: RX38.14, `afc20bbd73537d50108ca489a7ab8792cf65bbb3`.
Device result: the subsequent RX38.14 recording still becomes laggy and unplayable. Bandwidth declaration and bounded NACK repair did not resolve the sustained delay. See RX38-ARRIVAL-FEEDBACK-REVIEW.md. Host tests cannot establish Xbox encoder compliance or Vita network performance.

## What the new RX38.10 recording establishes

The supplied status/history/incident files identify RX38.10 revision `0286e874ab1ba5081e1d9b66325f5dbcaaa18bb7`. These are Home results, per the user. Raw recordings remain outside the public source repository.

- From 4–140 seconds, median decoded/shown rate is 60/60 fps, relative video arrival growth 3 ms, and the median receive-to-GPU average 66 ms.
- At 150.442 seconds, video payload reaches 4,179 kbps against the 2,000 kbps REMB request. By 152.449 seconds, all 277 receive passes hit the processing budget. Budget exhaustion is evidence of load, not proof that the OS socket itself contains seconds of backlog.
- Video arrival growth reaches 1,307 ms at 160.464 seconds, while the local receive-to-GPU average is 103 ms. Across 150–262 seconds, median video arrival growth is 1,012 ms, versus 4 ms for audio. The maximum observed video growth is 1,888 ms. The sustained problem is predominantly video-specific; it cannot be explained by display/GPU time alone.
- The incident ring retains 221.695–359.904 seconds: 739 compressed-queue pressure samples, five queue-capacity incidents, 12 completed recoveries (maximum 1,631 ms), and a maximum new-picture gap of 1,790 ms. This ring does not contain the entire session, and incident samples are not distinct failure counts.
- From 263–336 seconds, playback returns to median 60/60 fps and 1 ms added arrival delay. The adaptive target recovers. Another overshoot/delay episode follows around 336 seconds. The previous build's bitrate recovery change did not prevent these episodes.
- SPS remains 1280×720, output 960×544. The server's 3840×2160 metadata is not the decoded resolution. No new evidence supports changing decoder resolution or microphone behavior.

Relative arrival delay is measured against the fastest observed RTP arrival. It excludes fixed capture/network latency and is not capture-to-photon latency. Sender clock offsets are not treated as measured latency.

## Concrete implementation gaps and changes

### Bandwidth negotiation

The application previously sent maxBitrateKbps in startup JSON and subsequent REMB estimates, but never put a bandwidth declaration in its local SDP offer. The new typed RTC offer method adds `b=AS:2000` and `b=TIAS:2000000` only to accepted video sections, preserving audio, data channels, codecs, ICE credentials and rejected sections. Both initial and microphone renegotiation offers use it. The generated SDP remains the exact offer validated by the RTC state machine; arbitrary SDP mutation remains rejected.

This follows the bandwidth mechanism used by the inspected xcloud-player and Better xCloud source. Bandwidth declarations are requests, not evidence that an Xbox obeyed them. Subsequent REMB adaptation remains active with the same ceiling/floor and thresholds.

### Missing packet repair

The codec offer advertises generic NACK, but the existing interceptor chain only generates receiver reports. Previously the video reorder queue gave a hole six milliseconds and then abandoned affected compressed frames until a complete IDR arrived.

When the answer explicitly negotiates generic NACK for the video payload, the receiver now requests retained missing sequence numbers after a two-millisecond reorder grace, with retries no sooner than 20 ms. It retains at most 64 packets and waits at most 60 ms from the original gap, never extending that deadline on retries. Ordered traffic still passes immediately. Unsupported payloads retain the six-millisecond/24-packet policy. Released holes are never requested. Feedback is sent through the established SRTCP path; authentication/replay protection are unchanged. Remaining loss still uses complete-IDR recovery, never corrupted reference frames.

The 60 ms bounded repair window accommodates the recording's approximately 44 ms ICE round trip. It does not promise retransmission success on slower/lossier paths. Diagnostics report NACK queueing/failure and the offered SDP ceiling. The RTC dependency now invokes its previously unused authenticated incoming-RTCP accounting before interceptors consume sender feedback.

### Home Refresh

The previous action invoked `streaming.stop()` (including HTTP DELETE of the server session), followed by a new `/play` request. That implementation was inconsistent with its promise to preserve the running game, and matches the user's observed restart risk.

Refresh now sets a coalesced flag on the existing worker. That worker clears the partial video AU and held reorder packets, invalidates the old decode epoch through existing recovery, and requests a fresh keyframe. It does not close the peer, delete/start a server session, renegotiate chat, or change microphone/audio state. This repairs local video continuity; it cannot drain an uncooperative sender's remote queue. No automatic session restarts were added.

### Controls

A matching translucent quick-settings button occupies the bottom-right corner, opening the existing quick menu. Mic remains bottom-left and Xbox remains bottom-center. The Xbox menu row is removed. Touch routing uses the same geometry as rendering and keeps the entire gesture out of game input. The redundant top-right menu label is removed.

## Validation

All 148 host test executions passed in workflow 35819984251 (shared production tests execute in multiple suites). Native Vita compilation, VPK conversion, archive CRC/checksum and embedded revision/build verification passed. The executable has 117,180 bytes of metadata headroom against the required 65,536. Only eboot.bin differs from RX38.10. Full package identity is recorded in CANDIDATE-38.14-BUILD.json. Added coverage exercises bounded NACK behavior and wraparound, exact H.264 reconstruction after a 44 ms repair, unrepaired-loss deadlines, manual video resync, negotiated feedback filtering, bandwidth section scoping, real offer/answer handling, strict SDP validation, encrypted RTCP feedback delivery, and mic negotiation with the bandwidth limit preserved. Existing audio/input/decoder suites remain required.

Device acceptance: play Home through the prior failure window (preferably 10 minutes); confirm the payload rate/arrival growth and recovery behavior; use Refresh with a game in progress and confirm it stays in that game; verify all three touch buttons and voice. Repeat Cloud separately. A successful build is not a latency-clear result.

Primary protocol references: RFC 3264 (offer/answer bandwidth), RFC 3890 (TIAS), RFC 4585 §6.2.1 (generic NACK), RFC 8834 (WebRTC RTP feedback). No claim of server compliance derives from these standards.
