# Live-edge remediation issue register

Baseline: `8ea93088d2d1e8a0bb6cfff9eb43855ab5841adc`. This is question B
(client handling of stale media), not a claim to have resolved question A
(why media became stale before the measured socket).

| Classification | Evidence / mechanism | Correction and measurable gate | Risk |
| --- | --- | --- | --- |
| CONFIRMED CLIENT ARCHITECTURAL DEFECT | `CatchUp::observe` ignores `_delay_ms`; it requires eight queued frames. `MAX_LOCAL_VIDEO_AGE` starts at dequeue. Recorded 1.594690 s pre-socket growth therefore passes the client's age checks with a shallow queue. | Independent RTP/monotonic media deadline at receive, AU admission, decode and presentation. Replay must stop stale admission even with queue depth zero. | Legitimate jitter, clock drift and discontinuities must not be mistaken for permanent backlog. |
| CONFIRMED CLIENT ARCHITECTURAL DEFECT | `refresh_pending` continues the old reference chain and accepts a replacement IDR without any live-edge test. An IDR is not a timestamp reset or proof of current media. | One incident transition invalidates obsolete work; resume requires a complete current random-access AU and a matched current picture. Never rebase the media clock on recovery. | Lost/stale IDRs must leave an explicit unavailable state, not an endless PLI loop or stale playback. |
| CONFIRMED CLIENT ARCHITECTURAL DEFECT | Display selection checks only dequeue age; the surface can continue showing its previously uploaded texture after recovery. | Apply the independent deadline to repeated display too; make recovery visible even with diagnostics off. | Do not revoke a native texture lease or transfer decoder ownership unsafely. |
| ARCHITECTURAL WEAKNESS | Audio carries only local dequeue age; independent audio progression was stable during the recorded video onset. | Preserve independent audio timing; never delay current audio to match obsolete video. Propagate media deadlines without rewriting measured dequeue times. | Opus state, voice, mute and local output budgets must be preserved. |
| CONFIRMED FEEDBACK DEFECT, NOT PROVEN ORIGIN | `ReceiveBudget` considered a settled 1500 ms plateau healthy and raised the receive ceiling after ten seconds. Its regression test explicitly expected that increase. | Hold increases while media exceeds the ingress deadline; preserve the existing growth-based reduction policy. A plateau is neither spare capacity nor new evidence for repeated cuts. | This does not prove sender compliance with REMB or establish that this behavior caused the captured onset. |
| UNPROVEN HYPOTHESIS | Client feedback may influence sender latency. Capture proves local sending, not sender interpretation. | Keep truthful RR/TWCC reception and actual rendered-frame timestamps. Do not fabricate packet loss, timestamps, or proprietary health flags. Bound recovery-generated requests. | Suppressing genuine receipt feedback would distort congestion control. |
| UNPROVEN PHYSICAL CAUSE | Existing correlated packet bounds place growth before the observed socket, not necessarily outside Vita. | Keep this forensic question separate. A client cannot force unavailable current media into existence. | A no-stale-playback guarantee is not a liveness guarantee under an uncooperative sender. |

Acceptance distinguishes **safety** (obsolete pictures cannot become ongoing
multi-second playback), **conditional recovery** (a fresh decodable point is
delivered), and **availability** (actual Xbox/Vita supplies that point). Synthetic
IDRs are not evidence that a real sender will honor PLI. The existing real trace
contains timing observations, not complete H.264 payloads; replay cannot claim
to decode it or infer a counterfactual sender response.
