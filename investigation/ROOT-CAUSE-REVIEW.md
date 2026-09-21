# GreenVita latency investigation — OPEN

Follow-up: the subsequently supplied raw PTS trace and the output-servicing
candidate are documented in [DECODER-OUTPUT-CANDIDATE.md](DECODER-OUTPUT-CANDIDATE.md).
That update supersedes the earlier statement that no playback candidate exists.


Update: the requested simultaneous TV/Vita recordings and Test 34 history have
now been supplied. See [TEST34-CAPTURE-REVIEW.md](TEST34-CAPTURE-REVIEW.md) for the
new measurements, remaining causal gap and request for existing automatic traces.
The recording request below is historical and should not be repeated.

Reviewed 2026-09-21. This is an evidence review and diagnostic/test tooling change,
**not a candidate playback fix**. There is no verified explanation yet for the
remaining reported 5–6-second delay, and no new VPK is warranted by these findings.

## Exact source and binaries

- Repository: `jordanM333/green-vita`.
- Investigated branch: `vita-streaming-v2`.
- Investigated head: `d7397bf4ccbcce08b7df9069b083be1f15306471`, RX Test 34.
- Investigation work branch: `latency-root-cause` (based on that exact head).
- Original `vita-stream-tuning` stops at `752d4a91fb704adb559c11ccac94749c72840907`
  (Test 6); it is an ancestor, not the latest tested implementation.
- [Test 34 workflow](https://github.com/jordanM333/green-vita/actions/runs/35641383944)
  reports successful host gates and target packaging. This is build evidence only.
- Recovered Test 34 VPK SHA-256:
  `8ee5ece964b351a25ff264ac6ac467ff23113b5270fc9fffca2772ff04557ae1`.
- Recovered Test 33 VPK SHA-256:
  `5a01e5f8f525524069d004471f7b21bba0a29b4589c24788c9ef16af10f78f99`.
- Recovered Test 31 VPK SHA-256:
  `1f1088966e5f39adeadf3777b17784210631fca7d9764fe5ad5e2eff12d58eae`.

All 34 workflow runs, including failed/cancelled runs, are recorded with their
full SHAs in [build-history.json](build-history.json). Workflow run count must
not be confused with the number of successful installs. The supplied build-proof
image for Test 34 agrees with the Actions record; the latest recording visibly
says RX Test 34, Home, refresh:manual. No installed-file hash was collected on Vita.

## Evidence reviewed and its limits

Primary evidence recovered:

1. `pipeline-status.txt` and `pipeline-trace.csv`, uploaded September 20. The
   status has no embedded build revision. Its layout predates matched-picture
   diagnostics; exact binary attribution is **unknown**, not assumed to be Test 34.
2. Latest `IMG_0434.mp4` and `IMG_0434(1).mp4` (each 15.367 s, 30 fps), visually
   identifying Test 33 Home; `IMG_0444.mp4` (7.633 s, 30 fps), identifying Test 34
   Home. They do not show the Xbox display. The two 0434 files are visually similar
   but have different hashes; they are not independent repeat acceptance runs.
3. `IMG_0416.mp4` and the 15 earlier clips `IMG_0179`, `0180`, `0187`, `0188`,
   `0191`, `0192`, `0193`, `0194`, `0198`, `0199`, `0200`, `0202`, `0204`, `0216`,
   `0217`. All were screened using time-sampled frames; the relevant 0217 menu
   transition was then inspected at original frame cadence. This is not a claim
   that every control event or audio sample in every clip was measured.
4. Repository history, current source, vendored dependencies and existing tests.

The user's report establishes the unresolved symptom and its occurrence in both
modes. It does not by itself establish which internal stage first falls behind.
The short clips show gameplay, overlays and control interaction, but do not
provide repeated, unambiguous early/late button-to-response measurements of a
30-minute session. No numerical 5–6-second measurement has been fabricated from
an obscured button contact or a guessed game response.

### Directly measured legacy trace

The raw CSV contains 4,096 events from **78.667705 to 86.635585 seconds**: only
**7.967880 seconds retained**, not an 86-second measured playback history.

| Measurement | Samples | Median | p95 | Maximum |
|---|---:|---:|---:|---:|
| First received AU packet to decode submission | 481 | 0.142 ms | 6.560 ms | 16.806 ms |
| Individual decode call | 480 | 2.788 ms | 3.697 ms | 13.615 ms |

The first metric includes assembly and waiting for an output surface; it is not
an isolated compressed-queue measurement. This CSV lacks matched returned PTS,
matched receive-to-GPU timings and per-input delivery evidence. Its legacy
`picture_produced` identity is the current submission, which need not identify
the hardware's returned picture. It cannot establish output residence or freshness.

The status snapshot reports compressed queue 0/3, SDL audio 50 ms, audio RTP
buffer lead 20 ms, Opus queue 0, PCM queue 1, local input age 0/0 ms, and 28 audio
underruns/5 trims. These are mixed gauges and counters, not proof of smooth audio
or end-to-end response. Reported 4,078/4,077 ms sender offsets are uncalibrated
clock differences, not measured 4-second audio/video latency.

Reproduction: `python3 tools/analyze_latency.py --trace PATH/pipeline-trace.csv`.
The exact result is [legacy-trace-analysis.json](legacy-trace-analysis.json).

### Existing simultaneous TV/Vita evidence

`IMG_0217.mp4` already contains both displays, so a generic request to repeat that
old comparison would be unnecessary. In the first menu transition:

- TV transition begins in frame 12 (0.400 s), following a paused frame 11 (0.367 s).
- Vita transition begins in frame 16 (0.533 s), following a paused frame 15 (0.500 s).
- The same-camera onset difference is about **133 ms**, with a frame-sampling
  interval approximately **100–167 ms**, before extra rolling-shutter/visual
  annotation uncertainty. This is a display differential, not absolute input
  latency, network transit time, or a 100 ms acceptance result.

This event proves some additional delay on the remote-display path after a game
state already visible on the TV. It does not prove that input was prompt, identify
capture versus transmission versus decoding, or quantify the latest multi-second
failure. This older clip cannot validate Test 34 or a sustained run. `IMG_0199`
also shows both displays, but repeated movements without an exact build/session
anchor are not assigned a synthetic before/after latency result.

### Historical claims retained as secondary evidence

`STREAMING-V2.md` and `DRIFT-INVESTIGATION.md` summarize earlier pasted logs. The
full pasted histories were not recoverable in this turn; their tables are not
relabeled as newly verified raw data. They describe:

- Test 31 incoming-video collapse: relative RTP arrival growth rising from 1 ms
  to 2,466 ms while video throughput and decoded FPS fall; receive polling
  continues and individual decode calls remain short.
- Healthy decoder residence overlapping Test 31's 100 ms flush threshold.
- Earlier IDR-wait freezes and an expiry-before-gap-filler receive bug.

If accurate, the first pattern places a delay increase **at or before application
media arrival**, followed by old decoder output, rather than proving decoder
throughput caused it. These summaries do not distinguish sender pacing, network/
OS buffering, or a feedback interaction, and cannot be transplanted onto Test 34.

## Complete path and measurement boundaries

| Stage / source | What current code does | What is measured / missing |
|---|---|---|
| Physical control → SDL → `shell/mod.rs`, `input.rs` | Pumps SDL; reads current mapped state; samples during pacing | No physical-contact timestamp; source SDL revision in the build container is not pinned. Local A/B/RT marker is drawn later and can itself wait for UI refresh. |
| Application → `rtc/worker.rs` | Replaces latest state; coalesces unchanged state for up to 50 ms; retries latest rejected state and pulse release | `Input local` is sample-to-**successful admission**, not remote delivery. Failed periods can be absent from its average; suppressed samples are not button latency. |
| `control/input.rs`, `rtc/protocol.rs` | Encodes shared-origin monotonic input timestamps; controller and frame feedback share the channel | Admission success consumes sequence; no corresponding Xbox input-receipt or input-applied acknowledgement is decoded. |
| Vendored RTC/SCTP → UDP | Test 34 reserves capacity against association cwnd/rwnd, pending bytes and RTC ingress; then flushes | Outstanding bytes include in-flight data. No per-report actual UDP departure or Xbox receipt timestamp is exposed. Other channels share the association. |
| Xbox/game/capture/encoder | Outside local application | Unobserved by local timers. A TV comparison observes game response but includes the TV's display path. |
| Network/OS → UDP receive → SRTP/RTC | 32-packet/2 ms receive pass; authenticated media handed to app | Receive timestamp is assigned after user-space dequeue, not kernel arrival. No sender/receiver packet correspondence capture; queues before dequeue remain invisible. |
| RTP clock → reorder → AU assembly | Monotonic receive/RTP progression; 6 ms/24-packet reorder grace; damaged references wait for IDR | Relative RTP delay is growth against fastest observed arrival. Cannot detect a fixed baseline, pre-RTP capture delay, or prove unsynchronized absolute age. No silent-gap update until advancing RTP arrives. |
| AU queue → AVCDEC | Three AU slots, 50 ms local age policy; hardware initialization 1280×720, output 960×544 | Queue age/call time differ from returned-picture residence. Newer builds match returned hardware PTS to submitted records. Need matched timeline through failure, not just low call durations. |
| Decoder output → texture → SDL/GXM | Latest completed picture; bounded display callback completion | Matched receive-to-GPU exists in newer builds. Physical panel scanout is not measured. The compact `delay` field combines two different local measurements via max; it is not end-to-end latency. |
| Audio RTP → batch → Opus → PCM → SDL → hardware | Separate 80 ms RTP policy, batch channel, Opus/PCM queues and SDL queue; trims at threshold | Batch residence and queue counts exist; original RTP identity is discarded before PCM playback. No complete packet-to-audible age or calibrated A/V alignment measurement. Count limits alone do not prove media-age bounds. |

Cloud and Home share this client path, but use different remote endpoints and
network routes. A finding in one mode is not automatically a cause in the other.
All local durations should use one monotonic clock. Cross-device RTP/NTP offsets
must stay labeled uncalibrated. Use the camera's single timeline for physical
response; do not subtract Xbox wall time from Vita wall time.

## Change history and experiment disposition

No accumulated experiment is granted "fix" status merely because it passed CI.
The exact run/commit mapping is in the accompanying build history.

| Builds / relevant commits | Intended correction | Evidence and disposition |
|---|---|---|
| 1–3: `e524377`, `3d48db5`, `03873aa` | Request 540p; restore decoder allocation; accept valid smaller SPS | Request dimensions did not guarantee delivered dimensions. Preserve known-working 1280×720 allocation/960×544 output. No measured sustained latency improvement. |
| 4–6: `3567f42`, `d737829`, `752d4a9` | Reject incomplete AUs; decouple decode/display; latest texture | Useful integrity/ownership mechanisms; later replaced queue/recovery experiments must not be revived. Reported artifact improvement is not proof of latency resolution. |
| 7: `1b94759` | Home and rear-touch swap | Required functionality; preserve. |
| 8–10: `4de2b39`, `a5c2ef8`, `47bc11d` | Audio instrumentation/prebuffer/trim; Home exchange handling | Preserve Home exchange fix. Audio trims are an experiment requiring A/V and discontinuity review, not evidence against upstream audio lag. Do not expand them. |
| 11–12: `f51ecc4`, `7d4b08d` | Clocks/local input marker; diagnostics toggle | Preserve toggle and useful measurements, with corrected interpretation. |
| 13: `27b2447` | Session-monotonic input clock | Source-level correction. No paired gameplay result locating remaining delay. Preserve. |
| 14–15: `adb9b63`, `015a96b` | Correct texture lock/update; newest frame and skip unchanged paint | Preserve ownership correction; display scheduling remains a bounded local optimization, not seconds-long root cause proof. |
| 16: `1ecf3ab` | Receive work budgets and relative RTP timing | Does not bound kernel/network queue age. Preserve measurement; no further timing changes without evidence. |
| 17–20: `46347dc`, `00f0d62`, `f377543` | Receiver reports/negotiated REMB and SSRC binding | Failed intermediate runs are in history. Retain registration correction; bitrate compliance and live benefit remain separate questions. |
| 21–22: `15ebee1`, `ff16365` | Bounded reordering | Preserve complete-AU correctness, not a claim that all late packets are network loss. |
| 23–26: `7f8d67e`…`fe8e8b4` | Simplify recovery, deadline recheck, save on exit, process delivered gap fillers before expiry | Useful integrity fixes with host coverage; secondary historical trace reports IDR waits. Raw recovered trace is a different short healthy interval. |
| 27: `527b7ca` | Wake display on output; finish GXM callbacks | Addresses local scheduling; no sustained before/after gameplay acceptance. |
| 28–31: `a42576c`…`38f76ab` | Automatic reconnect/catch-up; PTS tracking; clearer HUD/menu | Automatic policies were a regression and workaround, not resolution. Preserve PTS tracking and manual user control; do not reintroduce automatic resets. |
| 32: `aa8f894` | Remove automatic flush/reconnect; report SPS restrictions | Preserve removal. Removing an interruption regression does not resolve drift. |
| 33: `3ed4f05` | Cap input/feedback outstanding payload | Reproduces old SCTP backlog under synthetic ACK loss, but its tiny ACK-based cap starves feedback at ordinary simulated RTT. Superseded; do not roll back to this admission policy as a proposed cure. |
| 34: `d7397bf` | Capacity-based admission and adaptive REMB | Admission starvation correction independently reproduced below. Adaptive bitrate model assumes a responsive sender/bandwidth bottleneck not established in live failure; it remains an unvalidated experiment. No new REMB tuning or rollback is performed during evidence capture. |

This investigation changes **no runtime playback code**. The appropriate cleanup
decision now is to exclude unsupported experiments from the eventual minimal
fix, not make an unmeasured multi-variable rollback before observing Test 34's
failure. In particular, adaptive REMB and audio trimming require evidence-based
retain/revert decisions; no demonstrated live root cause currently justifies
either as the final solution.

## Work implemented and validation in this investigation

Added `tools/analyze_latency.py` and eight tests. It separates camera response,
local call durations, matched output residence, and raw status; preserves missing
measurements as null; rejects mixed session clocks/duplicate trials; carries
camera annotation uncertainty; reports absolute latency and phase medians;
keeps idle/network recovery separate. It **never** upgrades low queues, stable
but sluggish response, uncalibrated clocks, or a short recording to acceptance.

Extended the existing SCTP virtual-clock tests to exercise **Test 34's capacity
rule** through repeated two-second ACK and bidirectional outages. The earlier
outage tests still used Test 33's superseded cap. These new tests target that
coverage gap; they do not reproduce the unresolved live failure.

Run on Linux with rustc 1.98.1, locked host dependencies and the repository's
vendored RTC/SCTP:

| Host scenario | Result |
|---|---|
| All transport tests, including full ICE/DTLS/SCTP API tests | 11 passed |
| Unbounded SCTP, two-second ACK outages | Reproduced 2,013 ms maximum report age, 16,254 B outstanding; recovered to 5 ms |
| Test 33 cap, healthy 100 ms RTT | 1,378 total deliveries; only 1 feedback admission |
| Test 34 rule, same 100 ms RTT | 3,624 total deliveries; 1,812 feedback admissions; max report age 50 ms |
| Test 34 rule, 100 ms RTT with specified brief loss | 3,576 deliveries; 1,812 feedback admissions; max age 50 ms; max delivery gap 48 ms |
| Test 34 rule, repeated ACK-only and two-way outages, 5 ms transit each way | Both: max delivered age 5 ms, tail age 5 ms; up to 5,418 B outstanding |
| Feedback negotiation/rate and adaptive-controller model tests | 6 passed; model is not Xbox sender validation |
| Offline evidence-analysis tests | 8 passed |

The 5,418 B value illustrates why outstanding bytes are not equivalent to stale
delivered content. Tests measure reports that arrive; they do not prove prompt
button delivery during an outage or Xbox game response. Full host WebRTC tests
and virtual SCTP tests have different scopes. Host ring/shared builds differ
from Vita patches as documented in the existing project.

Commands and outputs: [transport-tests.txt](transport-tests.txt),
[feedback-tests.txt](feedback-tests.txt), [analysis-tests.txt](analysis-tests.txt).
No new target compilation or VPK was performed because production code did not
change. No live Xbox/Vita session was available here. There is **no verified
platform limitation** and no measured best sustained end-to-end result.

## One focused capture needed — existing Test 34, no new installation

Question: **When Test 34 develops the reported seconds of delay, has the Xbox
already visibly acted, and do matched receive/decoder/presentation ages account
for the subsequent Vita delay?** The historical dual-display clip answers only
an earlier short event, not this revision's failure. The current Test 34 video
lacks the TV and a matching saved session timeline.

Use the already installed Test 34 in Home mode, with diagnostics on. Keep the
same network/settings and do not refresh. In one uninterrupted session:

1. Near the beginning, record the Vita controls/screen and TV together. Make
   three separated jumps or another clear action, waiting for each to finish.
2. Continue until the familiar delay appears; record the same three actions,
   leaving several seconds after release so both displays can catch up. Keep
   the overlay readable. A 60 fps recording is useful; 30 fps suffices to locate
   several seconds. Do not change camera mode between comparison clips.
3. Immediately hold SELECT 1.5 s → Exit game, then wait for the console list.
   This triggers the existing automatic log save. Provide that session's
   `pipeline-history.txt`, `pipeline-status.txt`, `pipeline-trace.csv` from
   `ux0:data/green-vita-540-test`, together with the two short clips as one bundle.
   Do not start another stream before collecting them. No fresh Cloud test is
   requested for this diagnostic split, and no 30-minute acceptance run is asked
   of an uncorrected build.

Interpretation is defined before seeing results:

- TV acts promptly but Vita acts seconds later: those extra seconds are on the
  media/capture-to-Vita branch, not wholly delayed controller delivery.
- TV and Vita both react late relative to the visible physical press: investigate
  the input/game side; the TV is not an Xbox input ACK and game/TV delay must be
  considered. Both paths may contribute.
- Large same-frame receive-to-GPU age: use matched PTS rows to separate residence,
  output/presentation, and receive/assembly delay. Small call time alone is insufficient.
- Large TV-to-Vita lag with small matched local media age: the missing time is
  before application receipt or outside its output measurement. Relative RTP
  progression and sender-report freshness can narrow this, not fully distinguish
  Xbox capture from network/OS buffers without additional endpoint evidence.

This capture has a specific decision purpose, not a promise that it can reveal
unobservable Xbox internals. No speculative fix is bundled with it. If it still
leaves indistinguishable upstream causes, state that explicitly rather than
substituting another settings change.

## Acceptance gate remains unchanged

- Two reproducible, uninterrupted runs of at least 30 minutes **per mode**,
  Cloud and Home, under stable network conditions.
- At least five clear button-response trials near beginning and end of each
  acceptance run. Report individual values, median/p95, absolute latency and
  added latency, plus measurement uncertainty. Initial drift target: at most
  100 ms. An inconclusive uncertainty interval is not a pass.
- A sluggish starting baseline fails; an absolute threshold must be justified
  from measured playable performance and agreed before acceptance. No unsupported
  absolute number is invented here, and stable 5-second latency cannot pass.
- No manual refresh, automatic reconnect, or periodic reset used to sustain
  responsiveness. Record actual decoder-error resets, IDR recovery, audio trims,
  underruns and disconnects; do not hide them behind averaged FPS.
- Current history keeps 600 once-per-second snapshots and the detailed trace
  keeps 4,096 events. Before a future acceptance candidate, preserve the full
  30-minute baseline/end interval with measured-overhead diagnostics or explicit
  early/end captures. The current ring alone cannot certify a 30-minute run.
- Audio synchronization, artifacts, freezes and controller presses/releases
  must not materially worsen. Camera/sound evidence is required where counters
  cannot establish it. Do not call SR V-A an audio/video sync measurement.
- Idle/resume and brief network-disruption recovery are separate labeled checks,
  outside the stable-network 30-minute runs; they cannot replace those runs.
- Candidate package must include exact implementation commit, measured mechanism,
  off-device reproduction, build hash, rollback VPK and the remaining hardware
  steps. Until hardware passes, label it **candidate fix—on-device verification
  pending**. At present there is not yet an evidence-backed fix candidate.

Current result: investigation tooling and host coverage improved; root-cause
localization and actual acceptance are pending. The issue remains OPEN.
