# RX38.20 failure review and RX38.21 correction

Status before packaging: implemented; final automated and native checks in progress.
**The user-visible progressive video-latency incident remains open until device acceptance.**
This review supplements `FULL-AUDIT.md`, including its full-path queue inventory,
clock contract, lifecycle review and frozen acceptance criteria. It does not
retroactively accept RX38.20 or treat its prior causal claims as established.

## Source and evidence identity

Existing repository `jordanM333/green-vita`, branch `latency-root-cause`.
Review baseline HEAD `a55dd5edbf51c0e6b190ca3ceaedeaabdb6afae5` adds documentation
after distributed RX38.20 source `80039b8ecbf078c6d8d45cb49b7938bb31f70c39`.
The supplied history and status identify that exact build/revision. The failing
recording is not a stale earlier-build recording. Remote installed bytes are not
accessible, but the prior package manifest and emitted identity agree.

The original scratch files were removed by workspace maintenance between turns.
The same repository and user files were recovered, their hashes rechecked, and
uncommitted corrections reconstructed and retested. No replacement project,
branch reset, account reset or unrelated source overwrite was performed.

| Supplied evidence | SHA-256 |
| --- | --- |
| pipeline-history 6.txt | eedfc20ab6a943e845f27e1ef8a0e1cd2133bf8ab9c32bfd223f225aab474372 |
| pipeline-status 5.txt | 1a7712c9529f6fd94ae2e86d940faea9e458c0466f77c3454b7a06d96475a134 |
| pipeline-incidents 6.csv | a17cfa6912c056b9ecf0ccb3a8db7cc684df23f626974787e15215d7257477bf |
| pipeline-trace 4.csv | 8d352f319200bd18e815ce3664ff78c553c8250a3d36887db31c50d1e7a61afb |

History spans elapsed 1.243–196.516 seconds, not a 30-minute test. Detailed trace
contains 4,096 rows over 194.640446–197.307806 seconds (2.66736 seconds). Mode was
not recorded: do not silently label this recording Home. Microphone is off.
RX38.21 records Home/Cloud explicitly; no account identifiers are added.

## New user comparison: .9/.10 versus current startup

User observed RX38.9/.10 play well for about 30 seconds, while recent Home builds
are laggy from startup. That is a real regression report, not device acceptance
for either old version. Verified workflow/source mapping:

| Build | Workflow | Source | Relevant differences |
| --- | --- | --- | --- |
| 38.9 | 35812410990 | 05720eee93134489db44f9809066ebb9822b02b5 | Working chat negotiation; receiver reports and adaptive REMB; no TWCC interceptor or NACK repair window. |
| 38.10 | 35816247205 | 0286e874ab1ba5081e1d9b66325f5dbcaaa18bb7 | Bottom controls, volume and settled-path recovery. Still no TWCC. |
| 38.14 | 35819984251 | afc20bbd73537d50108ca489a7ab8792cf65bbb3 | SDP bitrate constraint, negotiated NACK/60ms bounded repair, in-session Home refresh. |
| 38.15 | 35825348209 | 165f42963e1fe6d1754a6e836fd3ba06c1bc7e5a | Negotiated TWCC and first-packet codec binding, transport-load attribution. |
| 38.18 | 35960606896 | affe3c5e6f694fcb47063ba1454fc75fee53dace | TWCC activity disables receiver adaptation and resets/reopens the ceiling. |
| 38.20 | 36097525052 | 80039b8ecbf078c6d8d45cb49b7938bb31f70c39 | Fixed-offset fallback correction, audio-age and session-ownership safety; TWCC bypass remains. |

Inspected actual diffs of peer negotiation, transport, reorder, reports, session,
worker/decoder, UI and lifecycle. The normal in-order reorder path still returns
immediately; 60ms applies to a hole, not every packet. TWCC adds processing and
sender interaction, but no local video playback queue. There is no matched
.9/.10 versus .20 packet capture proving which change alone caused startup lag.
The new 38.20 recording's first video snapshot (3.246s) has 160 empty packets out
of 230 video packets, three submitted AUs, no decoder output yet, 170ms relative
arrival delay, a 59ms RTC-pump maximum and 140ms maximum AU assembly. This makes
probe handling relevant to startup; it does not establish a sole startup cause.

## Confirmed defects and corrections

| ID / affected code | Verified mechanism and consequence | Correction / validation |
| --- | --- | --- |
| F1 `rtc/feedback.rs` | Advancing video-extension and TWCC-send counters bypass `ReceiveBudget::receive` and force target back to 2Mbps. Activity was mistaken for effective sender adaptation. In .20: 1,908 TWCC sends, 389 REMB attempts, zero reductions despite growing arrival delay. The old test encoded a constant-offset scenario and never exercised active-TWCC growth. | Always evaluate the growth-based receiver constraint while continuing TWCC. Counter transitions no longer reset/release it. Unchanged video SDP during chat negotiation preserves it. A growth regression fails on affected code and passes after correction. Real encrypted transport test observes a reduced REMB at the peer while TWCC continues. This does not prove Xbox obeys it. |
| F2 `rtc/congestion.rs` | Rebasing the delay anchor every second forgives each small increase. Two successive >20ms/s increases were required; 1–10ms/s could grow indefinitely. A longer responsive-link model still accumulated ~24.6s with only F1 corrected. | Keep the delay anchor until a significant change, preserving sub-tolerance increases across sampling windows; drain clears the growth sequence. Keep existing ceilings, thresholds and feedback intervals. Slow-growth tests fail before/pass after. Constant offset and clock-skew regressions still pass. |
| F3 `rtc/rtp.rs`, `rtc/session.rs` | Empty RTP is ignored inside an unfinished FU-A, leaving a false sequence hole. A complete IDR and following P-frame are discarded and keyframe recovery requested. Also, probe timestamps are not necessarily media timestamps but previously entered the video clock. | Retain known padding sequence slots, ignore their marker/timestamp for media assembly, skip empty payloads only after sequence continuity is verified, and exclude them from media-clock/rate-control observations. Enforce the existing 2,048-packet bound even on padding. Real production assembler test fails before (two drops) and passes after. Missing fragments still fail; padding cannot finish an incomplete FU. |

359 empty video packets occur in the recording, but no packet payloads/sequences
are retained to attribute a measured fraction of actual recovery events to F3.
No claim that every empty packet is harmful or that all recovery is explained.

F1 removes an unsupported bypass, not TWCC itself. The sender still has its own
congestion estimate; REMB expresses an independent receiver constraint. The
default maximum remains 2Mbps, minimum 500kbps, video request 1280x720, decoder
1280x720, output 960x544. No arbitrary encoded-frame discard, new periodic reset,
smaller queue, new keyframe timer or lower fixed quality setting was introduced.

## What the evidence actually shows

Snapshot distributions below are percentiles of one-second observations/window
averages, **not frame latency distributions or input-to-display measurements**.
No-output decoder windows remain unavailable, not zero-latency samples.

| Metric, ms unless stated | p50 | p95 | maximum |
| --- | ---: | ---: | ---: |
| Relative video arrival | 108 | 1629 | 2014 |
| Relative audio arrival | 6 | 102 | 148 |
| Matched decoder window average | 77 | 133 | 6131 |
| Receive-to-GPU window average | 105 | 199 | 5006 |
| SDL queued audio | 81 | 120 | 148 |
| Local input admission average | 1 | 1 | 2 |
| AU queue count | 0 | 6 | 28 |

Timeline: video relative arrival grows from 823ms at63.312s to1560ms at68.318s
and1746ms at75.326s. At194.515s it is2014ms while audio is0ms, local input1/5ms,
decoder75/182ms average/max and receive-to-GPU103/220ms. The AU queue is empty.
This contradicts attributing that entire offset to the AU queue or GPU alone.
It cannot distinguish sender, path and kernel buffering; ingress timestamps and
synchronized capture/display clocks are unavailable.

Do not omit the long tails: at34.276s, decoder average/max6131/19812ms and
receive-to-GPU5006/19833ms occur after a19761ms video packet gap. At53.299s,
decoder1286/16916ms and receive-to-GPU1063/16914ms follow a16838ms packet gap.
Old firmware pictures after paused/static video are a plausible explanation,
not established without payloads and firmware access. This independent
pause/resume uncertainty is not closed by the feedback correction.

At193.512s RTP processing uses569ms for748 packets (6625kbps), plus69ms DTLS;
310/310 receive passes hit the2ms budget. At194.515s RTP366ms and DTLS136ms.
The receiver is busy. This supports addressing demand/control, not asserting
the network is solely at fault. The requested 2Mbps is not a measured wire cap.

At195.139284s a complete-IDR cutover removes23 queued AUs. At195.453996s the
32-frame limit still overflows, recovery starts, and an IDR arrives105ms later;
the new-picture gap is208.965ms. The final snapshot counts16 completed recoveries,
maximum2188ms, and6 completed refreshes. Retained incidents cover only the tail:
9 recoveries (median308ms/max2188ms),4 frame-limit events,82 picture gaps>50ms
(median61.557ms/p95 521.287ms/max2314.141ms). A final healthy-looking frame rate
does not negate those failures.

Detailed retained-trace per-event p50/p95/max in milliseconds:
decoder63.568/97.319/181.238; receive-to-GPU81.062/173.540/246.483;
decoded-to-GPU12.328/23.998/28.066; GPU wait3.309/5.219/5.496;
first receive-to-submit1.938/130.793/179.430. Physical scanout is unmeasured.

## Validation scope, regressions and risk

Tests use production feedback and H264 reorder/assembly modules. Before logs
retain the new regressions failing against .20. A 30-virtual-minute fluid-link
model includes a capacity reduction and a sender honoring REMB after100ms.
It is deliberately not Xbox/GCC emulation, hardware testing or faithful replay.
Its unchanged acceptance is <1000ms backlog after the first minute; the corrected
model's maximum is224ms, versus1,798,000ms with the fixed cap. This model alone
cannot establish the user's expected low-latency experience.

A separate 108,000-frame/30-virtual-minute test exercises exact fragmented bytes,
padding, reordered end packets, burst arrivals and sequence/timestamp wrap through
the real assembler. The hardware sink is substituted. Missing fragments, bad FU,
bounded padding, decoder backpressure, repeated recovery, native Opus, audio
stalls, input SCTP backpressure, mute/chat negotiation, lifecycle and catalog
tests are run by `tools/run_host_audit.py`. Clippy results include existing
warnings; a zero exit code is not a claim of no warnings or formal race proof.

New code adds no locks, threads, waits or shared mutable buffer ownership. Empty
packet storage stays under the existing AU count limit. Only target changes add
two incident records; the existing per-packet tracing and summary logging are
unchanged. Status adds mode. Runtime overhead on Vita remains unmeasured.

Regression risks: a sender that ignores REMB will not reduce delivered demand;
clock changes may look like growth; dynamic receiver limits may interact with
sender pacing. Fixed-offset, pause, wrap, jitter and slow-growth tests constrain
these risks, but do not prove Xbox behavior. Strong persistent overload below
the500kbps floor is not claimed solvable by this controller.

Home refresh remains media-only: `refresh_home_stream` → streaming refresh →
coalesced RTC flag → intact-IDR video cutover. It does not invoke REST stop/play,
quit, power-off, or reconnect. Existing Home detach versus owned Cloud stop
semantics and no-unowned-session-sweep protections are retained and tested.
No voice capture/Opus/volume/mute, control mapping, rear-touch swap, catalog,
account, bottom buttons or diagnostics-toggle logic was changed. Their device
behavior, including echo and voice-on performance, is not claimed reverified.

## Frozen acceptance and one device procedure

Retain all limits in `FULL-AUDIT.md`:30min Home and30min Cloud; early/late p95
growth≤50ms; no added delay>250ms sustained5s; no measured action-to-visible
response>500ms or recurrence of5–6s lag; audible A/V relationship within100ms of
baseline after recovery; isolated-disturbance recovery≤1s without repeated
storms; no game termination on Home refresh; retained features working.
**Actual device duration here:0 minutes.** These criteria have not been relaxed
to pass the model or the build.

Use one candidate with unchanged network/settings. Home: play30min, include an
in-game pause/resume and one manual refresh, enable/disable voice and diagnostics.
Check action response near startup and minutes2,5,15,30. Repeat Cloud on the same
candidate. Stop early if clearly unplayable; do not keep restarting to obtain a
favorable sample. The app automatically retains history/status/incidents and
the tail trace. New `receiver_ceiling_bps` / `receiver_ceiling_delay_ms` incidents
and explicit mode let us check whether lower advertised demand is followed by
lower received rate and draining delay. Sent feedback is not acknowledgment.

Pass establishes sustained behavior only for the tested device/path. Failure with
a falling cap but unchanged rate challenges the sender-response hypothesis;
failure after rate/arrival settle shifts focus to local decoder/presentation or
input. The 19-second silence/tail issue needs pause/resume observation, not a new
arbitrary threshold. Source/workflow/VPK identity, final test counts and rollback
will be recorded in `CANDIDATE-38.21-BUILD.json` after successful packaging.
