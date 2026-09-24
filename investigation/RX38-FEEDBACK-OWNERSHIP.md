# RX38.16 rejection: congestion-feedback ownership

The user reports that RX38.16 is no different. It is **not accepted for latency**.
No new recording accompanied this report. The most recent full recording is
RX38.15; do not label it an RX38.16 recording or claim a device result for this
correction. Raw recordings remain outside the repository.

## Evidence and reproduced defects

The RX38.15 recording already proves that arrival feedback was active: two
bound TWCC tracks, 41,181 authenticated transport extensions and 2,858 sent
TWCC datagrams. At 290.229 s, added video arrival delay is 1,991 ms, ICE round
trip is 2 ms, and average local receive-to-GPU time is 190 ms. The compressed
queue is empty at that sample; a later catch-up burst overflows it. These are
relative arrival measurements, not synchronized capture-to-display latency.

The same session still runs the legacy REMB controller. It reduces the target
by 30% each second when its window observes at least 100 ms added delay and
only recovers below 80 ms. A fixed 200 ms offset, with no further delay growth,
reproducibly drives the target from 2 Mbps to its 500 kbps floor. The returned
recording also reaches that floor. This absolute-offset loop continues even
when the sender already receives transport-wide arrival timing.

This is a confirmed conflicting control path, not proof of Xbox's internal
response. The upstream WebRTC implementation applies receiver bitrate limits
to its send-side estimate:
https://webrtc.googlesource.com/src/+/refs/heads/main/modules/congestion_controller/goog_cc/send_side_bandwidth_estimation.cc
Xbox may implement adaptation differently. Incoming video sometimes exceeding
REMB is not an acknowledgment of the ceiling, nor proof that the ceiling has
no effect on its target or pacing. We cannot identify the exact upstream queue
from these logs alone.

RX38.16 also requests a keyframe every five seconds on sustained arrival delay
even with an empty local decoder queue. The new regression initially failed:
an empty-queue stream with a fixed 1,500 ms offset requested IDRs at 500,
5,500 and 10,500 ms. A keyframe cannot bypass an upstream sender/network queue
and adds burst traffic. Requesting it repeatedly without a local catch-up
opportunity was not supported by the returned evidence.

## Correction

* When authenticated **video** transport extensions and successfully sent
  TWCC datagrams both advance within two seconds, suspend legacy REMB rate
  adaptation. REMB retains the original 2 Mbps ceiling while the sender's
  transport-feedback controller adapts beneath it. This is not a request for a
  fixed 2 Mbps stream, an increased maximum, or a reset of measured delay.
* Negotiation alone, audio-only traffic, malformed/wrong extensions, and
  historical counters cannot enable this mode. If either activity stops for
  two seconds, use the existing legacy controller with a fresh estimate. On
  activation, send the restored ceiling immediately instead of carrying a
  previous 500 kbps clamp. A new SDP answer resets the mode.
* Automatic keyframe requests require the existing sustained local queue
  pressure. Arrival delay alone no longer generates requests. The five-second
  cooldown, safe complete-IDR queue cutover, actual damage recovery and
  session-preserving manual Refresh remain intact.
* Status identifies active ownership as `REMB:twcc-cap`, with the actual
  ceiling and unchanged arrival-delay observation. The legacy mode remains
  `REMB:adaptive`. Sent feedback is still not a server acknowledgment.

Microphone capture/Opus/mute, voice/game levels, bottom buttons, catalog,
packet-repair limits, hardware decode, power settings and session teardown
are outside this change.

## Verification

The production feedback tests cover a ten-minute simulated busy/paused/resumed
scene with a persistent 1,991 ms offset: no competing REMB reductions, while
the reported delay remains visible. Additional tests cover releasing an old
floor clamp, failed/stalled or audio-only feedback, fallback and SDP reset.

The encrypted ICE/DTLS/SRTP/SRTCP regression uses the production feedback
policy and receiver interceptor. It observes real decrypted TWCC, receiver
reports and repeated 2 Mbps REMB at the remote peer despite a supplied delay
offset. A peer declining transport-cc still receives media and receiver
reports. The complete local transport suite passes 43 tests, including voice
negotiation/mute and input/backpressure regressions. The keyframe-policy suite
passes all four tests after the red-to-green empty-queue regression.

All 185 host test executions passed in workflow 35960606896, including
decoder/refresh safety, RTP ordering, catalog, native Opus and analysis. The
native release build, VPK conversion and ELF gate passed; metadata headroom
is 89,948 bytes against a 65,536-byte minimum. Artifact/VPK SHA-256, both ZIP
CRC checks, embedded revision/build and GRNVTEST1 identity were verified.
`CANDIDATE-38.18-BUILD.json` records the complete identity and result.

RX38.18 source is affe3c5e6f694fcb47063ba1454fc75fee53dace. The intermediate
38.17 was superseded before delivery: final review caught that a new SDP
answer must retain counter baselines while clearing activity timestamps.
Otherwise historical counters could briefly reactivate TWCC mode. The
regression now verifies that unchanged counters and audio-only advancement
cannot reactivate it, and fresh video plus transmit activity can.

This correction removes two demonstrated feedback defects. It does **not**
establish that all sustained Home or Cloud delay is fixed on a Vita. Device
acceptance requires play through the previous failure window, including
in-game pause/resume, with stable response and no growing arrival/local queue
delay. Home and Cloud require separate results. Do not mask a failed result
by resetting the timing baseline or increasing keyframe frequency again.
