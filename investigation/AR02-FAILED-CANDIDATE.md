# AR01-4 hardware failure investigation

Baseline: `68dc5032c6751a1e2ce0282c6eb27df0a61ae6e3`. The user's unplayable experience fails acceptance, regardless of host/CI results.

## Issue register before changes

| Classification | Evidence and mechanism | Correction and verification | Risk / uncertainty |
| --- | --- | --- | --- |
| CONFIRMED RECOVERY DEFECT | Current Home session: manual_refresh at 142.401615 s with 1480 ms added receive delay. A replacement IDR is admitted by 144.204 s, but receive delay remains 1456 ms and ends at 2023 ms. The menu only calls VideoRtp::refresh, preserving the RTC transport and sender media path. A keyframe repairs dependencies; it cannot clear work before the receive point. | Give the Home menu an actual media reconnection on the **same owned Stream**, join the old worker before replacement, cancel old ICE jobs, clear old audio/video state, and show connection progress. Test request routing, repeated activation, stop during refresh, and failure. | Xbox must accept a fresh SDP exchange on an existing session. Host tests cannot establish that server behavior. No /play, /connect, DELETE, power or game commands belong to this operation. |
| ARCHITECTURAL WEAKNESS | Requested video 2000 kbps; after delay begins, advertised REMB falls to 500 kbps while received video repeatedly exceeds 2500–4900 kbps. Sending feedback does not prove sender compliance. | Audit SDP, RTCP wire and feedback architecture; do not declare the sender controlled from queued/sent counters. | Cause of failure to constrain sender remains unproven. Do not replace that uncertainty with another arbitrary bitrate. |
| CONTRIBUTING DEFECT | Prior tests exercised local queues, RTP assembly and IDR cutover. They did not exercise the menu's complete recovery lifecycle or reproduce the upstream delay. | Add actual backend lifecycle tests. Keep device acceptance separate from host policy tests. | Synthetic RTC senders do not implement Xbox pacing/encoding. |
| NOT SUPPORTED AS SUFFICIENT ROOT CAUSE | SCTP forward-TSN storm is absent: current session FWD tx=0, T3=0; still unplayable. | Retain the independently verified SCTP fix, withdraw it as a sufficient explanation of the user's streaming failure. | It was a real protocol defect, not proof of the whole diagnosis. |
| UNPROVEN HYPOTHESIS | Sender encoder/pacer or network scheduling accumulates video work before application delivery. Current session 92.127→95.129 s: video offset 1→1367 ms, audio 15→11 ms, RTC max calls 3–5 ms, decoder queue snapshot zero, no missing RTP after reorder. | Further protocol/source investigation and measured convergence are required. | No complete packet capture or sender telemetry exists. Socket queues and physical input-to-panel latency cannot be reconstructed from these logs. |

The current session stays near its initial arrival timing through approximately 90 s; the previous session first exceeds 900 ms by 19.148 s. These are different sessions, not evidence against the user's report of rapid failure. History snapshots are one-second samples; detailed trace rings retain only the tail. Process-wide decoder/renderer counters carry over and must not be summed as session counts.

Release gate remains **FAILED** until a correction explains the sustained-delay path. Compilation and passing synthetic tests alone do not authorize calling another package a streaming fix.

## Implementation and test scope

Home Refresh now retires its local RTC worker, joins decoder teardown, cancels prior ICE jobs, clears candidate deduplication, and creates a new worker using a clone of the same owned `Stream`. The session path/credentials and microphone preference are retained. Changing the output identity invokes the existing shell audio reset and texture-detachment paths; the app discards old pending audio and old video timing. UI progress is immediate, repeated requests coalesce, errors are visible, and a missing first picture releases the retry lock after 15 seconds. The automatic packet-loss/IDR path and Cloud behavior are unchanged.

The actual backend is compiled into the lifecycle harness with a fake native-worker boundary. It tests 100 refreshes, coalescing before the first picture, exclusivity of decoder ownership, stop during retirement, allocation failure/retry, and unchanged Cloud cleanup. The actual Stream HTTP boundary separately tests 100 repeat SDP exchanges restricted to POST/GET on the same owned session's `/sdp` endpoint. **This does not prove the real Xbox accepts repeated offers.** The former backend fails the new regression at the unchanged audio/video generation assertion; the corrected backend passes. A separate virtual-clock test prevents permanent retry lockout after a missing first picture. Final review found that a late worker error or closure could bypass retry handling after that timeout. Recovery identity now survives timeout/failure until the replacement produces a picture; tests cover repeated error/closure, explicit retry, and normal closure after the first picture.

Existing host gates passed 260 Rust test executions, 27 Python tests and all 11 strict Clippy harnesses before the final timeout/signaling tests; the final lifecycle harness has 15 passing tests (previously 6). Native compilation and strict Clippy passed for the initial recovery change in workflow 36289963422. The final late-failure handling revision requires a new complete CI run; its results must be checked separately.

## Sender-control review and remaining falsifiable hypotheses

The input metadata layout agrees with the primary xbox-xcloud-player implementation (repository checked out at the recorded reference HEAD in the evidence workspace). The RTC offer reparses its bandwidth-modified SDP, and the existing encrypted two-peer tests confirm delivery of RR/TWCC/REMB. No evidence was found that simply increasing the decoder queue, dropping more decoded frames, or requesting another IDR would clear the pre-receive delay.

Primary WebRTC sources demonstrate two relevant mechanisms, **not proof of Xbox's implementation**:

- [GoogCC packet-feedback-only mode](https://chromium.googlesource.com/external/webrtc/+/eb73a7bd16c725477ca2da6dc0e6fea236616d44/modules/congestion_controller/goog_cc/goog_cc_network_control.cc) explicitly ignores REMB in that mode. Local REMB counts cannot establish encoder adaptation.
- [Legacy WebRTC pacer](https://webrtc.googlesource.com/src/+/068a2e380bebc2a0aef86f2c5e85b7ebcfb67095/modules/pacing/paced_sender.cc) permits a two-second queue horizon and a pacing multiplier. The similarity to the observed video-only delay is a hypothesis, not a measured sender queue.

Neither disabling TWCC nor arbitrarily raising the bitrate has been applied: the actual sender contract and sustainable Vita/network capacity are not established by these logs. There is no complete encrypted/decrypted packet replay, no sender encoder/pacer trace, and no synchronized physical presentation reference for these sessions. Those absent observations prevent a defensible claim that the sustained streaming root cause is fixed.

Packaging now requires an explicit release-candidate marker in addition to host/native gates. Investigation pushes run compilation and analysis without automatically emitting a VPK. No new hardware candidate is authorized by passing compilation alone. Rollback remains AR01-4 source `68dc5032c6751a1e2ce0282c6eb27df0a61ae6e3` (itself known to fail gameplay acceptance).
