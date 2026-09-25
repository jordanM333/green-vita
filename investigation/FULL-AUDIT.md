# GreenVita full audit and remediation

Audit started 2026-09-25. Baseline branch `latency-root-cause`, clean source
`f8f069cff510bbda5fee992f331d8e24a0f30af2`. No replacement project or broad rewrite.
This is a working record; uncompleted checks are not passes.

## Acceptance criteria (frozen before remediation testing)

Host tests must preserve decoded-reference dependencies, reject stale work after
cutover, handle sequence/timestamp wrap and reordering, keep memory bounded under
overload, and terminate workers without deadlock. A synthetic sender/decoder is
not a reproduction of Xbox or Vita hardware. Regression tests must exercise the
production mechanism, not merely assert its constants. Tests that inject stalls
must measure age from original admission, never reset age at a later handoff.

Hardware acceptance is pending unless explicitly recorded otherwise. Run Home
and Cloud for at least 30 minutes each. Compare minutes 2–5 with minutes 25–30:
relative-arrival and receive-to-GPU p95 must not grow by more than 50 ms; no
sustained added media delay above 250 ms for 5 seconds, no action-to-visible
response above 500 ms in the annotated checks, and no recurrence of the reported
5–6 second lag. These are engineering regression limits, not claimed Vita specs.
The 50 ms trend tolerance permits ordinary short jitter without accepting
progressive accumulation; 500 ms explicitly rejects an unplayable interaction.
Report p50/p95/p99/max, duration and trends, not only pass/fail or final values.

Audio must not retain old local work after a stall; SDL queue must stay within
its existing 240 ms safety bound. Actual audible latency must not progressively
grow and must return within 100 ms of the initial measured A/V relationship after
recovery. External camera/audio observation is required for that claim.
After an isolated disturbance, playback must become current within one second
after delivery and hardware service return to normal; ongoing packet loss is
reported separately, not counted as successful recovery. No automatic session
restart, repeated refresh loop or recovery storm on an intact reference chain.
Manual Home refresh must cause no REST stop/play, quit or power command; the
same running game/session must remain on-device. Voice on/off, diagnostics
on/off, Guide/mute/quick settings, rear touch mappings, account/catalog, start,
exit and start again must continue functioning.

## Evidence classification

- User observed: initially flawless play, progressive Home delay and audio lag,
  Cloud delays, earlier Home refresh restarting the game, working voice without
  reported echo. Office guest Wi-Fi performed poorly. Those observations do not
  identify the buffering component.
- Recovered latest distributed package: RX38.18, SHA-256
  `b231093897725264b08124eb25aaa0e5305cc83e8ade49d40e55e7e4459bb79e`.
  Workflow 35960606896 succeeded, head `affe3c5e6f694fcb47063ba1454fc75fee53dace`.
  Baseline HEAD only adds its build record. Actual currently installed bytes are
  not remotely accessible. No RX38.18 device recording has been supplied.
- Supplied recent history/status files identify RX38.14 and RX38.15. Metadata
  traces contain no encrypted/decrypted media payloads, so faithful packet or
  decoder replay is impossible from these files.
- Local receive-to-GPU is not input-to-display. GPU completion is not a measured
  display scanout. Relative RTP arrival delay can include sender/path/socket
  buffering and clock skew; it does not locate the queue. Absolute SR offsets
  require clock synchronization that has not been established.

## Executive finding

Status: **implemented and host-automated-tested; native packaging and hardware
acceptance recorded separately in the release manifest. The primary progressive
latency incident is not closed.** There is no accessible Vita/Xbox test rig.

The evidence establishes multiple defects, not one established explanation for
all symptoms. Older queue-overflow/IDR recovery caused freezes; audio handoffs
could preserve old audio while losing its age; unowned session cleanup could
terminate another attachment; unmatched decoder pictures could inherit an
unrelated input epoch; fallback bandwidth control repeatedly penalized a fixed
arrival offset; chat renegotiation could leave feedback policy stale. Corrections
below address these mechanisms without another queue-size/resolution change,
automatic reconnect, periodic reset or arbitrary dependent-frame discard.

What remains unknown is where all of the user-observed 5–6 seconds accumulate in
the currently installed build. There is no synchronized input/console/Vita
recording, no capture-time clock mapping, no kernel ingress timestamps, and no
RX38.18 recording. Successful tests of individual mechanisms cannot fill those
gaps. A flawless startup or a good final snapshot is specifically not acceptance.

## Current source, binary and history

Repository: `jordanM333/green-vita`; active branch `latency-root-cause`.
Baseline source tree: `ffaa2df96753f3b256ea799ffa5cb9b3b71ad3c0`.
RX38.18 source tree: `60b419bca0c6bc59e1281a0fb3e7781ae08c8a8e`.
Baseline HEAD is the release-record update after RX38.18, not an unbuilt code fix.
Recovered package executable SHA-256:
`4b99d51c3957e1c1e75392f79a20348b2617305236ca741b854f8fe349389683`.
Its actual executable contains both `38.18` and the full `affe3c5...` revision.
GitHub workflow 35960606896 independently identifies that checkout and success
(2026-09-24 05:35:14–05:44:43 UTC). SFO title ID is `GRNVTEST1`; APP_VER and
VERSION are both `00.00`. Those SFO fields do **not** identify a test build.
The next workflow checks embedded strings, checkout SHA, tree, tracked-source
cleanliness, Cargo.lock hash, SDK digest, executable hash and package hash.
It does not establish what bytes are installed on a remote Vita.

Meaningful history was reconstructed from source diffs, build-history.json,
CANDIDATE manifests and the incident reviews, not from their conclusions alone:

| Change/revision | Mechanism and evidence | What it did not establish |
| --- | --- | --- |
| `a42576c`, `38f76ab`, `aa8f894` | Home reconnect/decoder recovery evolved; automatic restart loop later removed. | A reconnect's effect on a live console game was not proven harmless. |
| `3ed4f05`, `d7397bf`, `cbadd95` / Test34 | SCTP admission, feedback starvation, returned-PTS association. Current real-SCTP tests reproduce >1.9s old reports with unbounded admission and feedback starvation with the old ACK cap. | This proves earlier transport defects, not current Xbox input-to-display latency. |
| `5185b47`, `1b88867` / 35.x | Separate AVC output polling; stop pointless empty polls. Host fake hardware exercises delayed outputs and returned PTS. | Firmware's output-only service and hidden buffering still need real hardware. |
| `9936ade`, `70ccd93` / 36.3–37.4 | Larger bounded AU admission, intact reference-chain handling, scheduling/input priority. Original trace has six-AU overflows and repeated IDR waits. | Avoiding a local overflow cannot bound sender/socket/firmware delay. User still reported failures. |
| `90c472f`, `37b5c01`, `05720ee` / 38.x | Account collections, mic capture, Xbox chat negotiation and audio-transceiver direction reuse. | Working voice is user-observed, not evidence that video latency is healthy. |
| `0286e87` / 38.9–38.10 | Consistent bottom controls, volume and settled-path recovery. | Low SDL byte count alone does not establish fresh audio. |
| `f758fad`, `9d71b77`, `afc20bb` / 38.14 | Home in-session refresh; negotiated bitrate/NACK and real SRTCP counters, valid SPS fixtures. | Packet counters do not prove sender adaptation. RX38.14 still has ~1.8s arrival growth. |
| `165f429` / 38.15 | TWCC interceptor, first-packet binding and traffic classification. | RX38.15 still has an 8.343s recovery and ~1.991s relative arrival growth. |
| `297174d` / 38.16 | Nonblocking keyframe catch-up, retaining live decoder. | No supplied sustained device acceptance for this revision. |
| `4a3881d`, `affe3c5` / 38.18 | One feedback controller when TWCC is active; refresh feedback after initial SDP. 38.17 was canceled. | REMB-only fallback and later chat answers still needed the corrections in this audit. No RX38.18 device evidence. |

Thus previous improvements often corrected a **local** defect while leaving
other buffers, ownership paths or the actual sender response unverified. The
old evidence cannot be used to claim the current binary has the same failing
code, nor can later source changes be used to claim the observed incident gone.

## Evidence measurements and timeline

Recovered RX38.14 history: 421 non-startup/inherited snapshots spanning 425.628s;
RX38.15: 286 spanning 289.363s. Neither is 30-minute acceptance. Below are
percentiles **of one-second snapshots/window averages**, not frame percentiles:

| Measure (ms except queue) | RX38.14 p50 / p95 / max | RX38.15 p50 / p95 / max |
| --- | --- | --- |
| Relative video arrival growth | 4 / 1478 / 1837 | 2 / 1408 / 1991 |
| Relative audio arrival growth | 7 / 163 / 222 | 153 / 161 / 175 |
| Window-average matched decoder residence | 56 / 102 / 300 | 58 / 122 / 2933 |
| Window-average first-receive→GPU completion | 69 / 159 / 464 | 70 / 190 / 2262 |
| SDL queued duration | 85 / 109 / 149 | 76 / 118 / 153 |
| AU queue depth | 0 / 2 / 24 | 0 / 2 / 26 |
| Local input admission average | 1 / 1 / 2 | 1 / 1 / 2 |

Recovery maxima: 1597ms / 8343ms. These are retained running maxima, not
distributions of individual recoveries. Decoder first-to-last valid window
change was −1ms / −112ms, which conceals intermediate stalls if taken alone.
At RX38.15 elapsed 290.229s: video relative arrival +1991ms, audio +159ms,
AU queue 0, local decoder average 136ms/max295ms, receive→GPU average190ms/
max380ms, local input average1ms, target500kbps. This contradicts identifying
the AU queue alone as the source of that arrival offset. It does not distinguish
sender frame timing, path queues or already-buffered kernel datagrams.

The retained RX38.15 detailed trace is only 2.968073s (290.877331–293.845404s,
4096 rows). Its per-event p50/p95/max (ms): receive→submit .139/6.846/30.689;
decode-call 2.917/8.228/10.054; poll 4.317/8.235/10.759; matched decoder
57.637/92.143/110.196; receive→GPU 74.849/114.867/134.854;
decoded→GPU 12.879/25.126/27.728; GPU wait 3.438/5.093/5.368.
Those values are not evidence that the earlier stall never occurred.

An explicit old user-trace freeze: at 72.372807s an incomplete AU is abandoned;
the reference chain is gated and keyframes requested about every300ms. A
complete IDR arrives at74.080358s (1707ms recovery wait), old decoder pictures
are retired, the current IDR is returned at74.159917s and GPU completes at
74.165652s. The new-picture gap is1820.572ms. Presentation/GPU continued during
the freeze but showed the old image. Another trace segment reaches AU depth6,
discards an intact dependent chain on overflow and repeats the recovery cycle.
Current32-AU admission/IDR-safe cutover predates this audit; those events must
not be described as a new reproduction of the current implementation.

Videos IMG_0434 (15.367s) and IMG_0444 (7.633s) were inspected visually. They
show the handheld/game but no synchronized console/reference display or
unambiguous input-response markers. No numerical end-to-end result is inferred.

### Clock and measurement contract

Application handoffs use the same process `Instant` monotonic domain. Video RTP
90kHz, audio48kHz and sequence16-bit wrap are handled independently. Extended
decoder PTS identifies returned pictures; input-call identity is not substituted.
RTP deltas and earliest-arrival-relative growth avoid equating remote clock
epoch to local epoch, but sender rate changes/discontinuities/skew remain
possible confounders. Unsynchronized SR offsets/V−A are not capture delay or A/V
sync. Audio age now starts at first post-decryption packet admission and survives
SampleBuilder/Opus/PCM. It is not kernel ingress or microphone capture time.

Stage meanings: `au_first` is first observed AU packet; `au_complete` is
validated assembly; `decode_submit` precedes the firmware call; returned PTS
provides completion identity; `picture_produced_output` is decoded output;
`present_return` is display submission return; `receive_to_gpu_done_us` follows
GPU completion. Physical scanout, speaker DAC and Xbox controller processing are
not measured. Do not add independently windowed averages into an E2E estimate.

## Whole-application coverage and queue inventory

Source-level review covered app entry/authentication, settings/storage, account
catalog and image workers, controller/touch/UI, session lifecycle, RTC negotiation,
transport/feedback, H264 assembly, AVC worker/FFI/output surfaces, graphics and
audio/mic paths. Host harnesses compile selected production modules; only the
native build type-checks the full Vita binary. This is not a formal proof of
all application behavior or a third-party-dependency security audit.

The table distinguishes a byte/count bound from a **wall-clock residence bound**.
An indefinitely stalled consumer can make even one retained item arbitrarily old.

| Buffer / producer → consumer | Capacity / overflow | Residence and stalled-consumer behavior |
| --- | --- | --- |
| Sender capture/encode and network devices → Vita socket | Remote/OS capacities unknown | No application bound. No kernel-arrival timestamps in supplied logs. Ethernet into an extender is not proof of a wired path to router; throughput is not a latency measurement. |
| UDP socket → RTC pump | 2048-byte receive buffer; pass budget32 datagrams or2ms | Pass budget is fairness, not socket capacity. No established max kernel residence. Reads timestamp after dequeue; send flush also awaits socket writes. |
| RTC/DTLS/SRTP/SCTP internals → media/control handlers | Dependency-owned buffers; SCTP congestion window/flight separate from immediate-send capacity | No global wall-age guarantee from packet counters. Actual dependency exercised in host peers; OS/remote behavior not emulated. |
| RTP reorder → H264 assembler | 24 packets/6ms without NACK;64/60ms negotiated repair | Gap expiry when serviced, not while thread suspended. Late duplicates discarded; NACK after2ms then20ms intervals, within60ms repair window. |
| H264 fragmented AU → complete AU | One AU;2MiB/2048packets limits | Incomplete AU waits for subsequent packets/timestamp; no unconditional wall expiry during silence. Damaged AU discarded, dependent chain gated until complete IDR. |
| Complete AU → AVC worker | 32 AUs AND4MiB reservations | Finite memory, not finite firmware stall. QueueFull triggers one damage epoch/IDR gate.50ms pressure is diagnostic, not arbitrary encoded-frame shedding. |
| Decoder PTS bookkeeping → returned-picture lookup | 256 timing entries | Metadata capacity is not firmware capacity. Returned identity required; unmatched output now rejected rather than attributed to current submission. Firmware residence unbounded by this table. |
| AVC/driver output → renderer | Three leased CDRAM surfaces; one latest pending picture | Superseded decoded pictures can be dropped safely. Decode target cannot alias displayed/pending surface. In-flight lease must finish before surfaces freed; teardown can wait on a hung firmware call. |
| Renderer → GPU/display | Per-present GPU-completion synchronization | Measured GPU callback wait, not scanout. Old displayed frame can remain indefinitely with no new video. UI capture can be delayed by a blocking render iteration. |
| Audio SampleBuilder → RTC batch | max-late32 packets/80ms RTP-time policy; timestamp metadata128 | RTP-time policy does not bound wall age during silence. Original first-admission age now retained even across timestamp wrap and release after a stall. |
| RTC audio batches → session/UI | 16 bounded batches, normal receive-pass batching; old batch gate | Full mailbox drops new batch. Existing batch age is not sufficient after handoff; per-packet age now continues downstream. |
| UI → Opus decoder | 32 packets; nonblocking admission drops new on full | Count corresponds to640ms for20ms packets, not a wall bound. Decode advances prediction; PCM from expired packets is withheld. |
| Opus → UI PCM | 8 decoded buffers | Was a blocking send, permitting backpressure through the audio chain. Now nonblocking; full sink drops PCM, original age retained and rechecked by renderer. |
| PCM → SDL audio | start40ms; trim160→80ms; hard240ms | Original age + queued duration + new PCM duration must fit240ms at admission. Service stall clears SDL. Cannot retract samples already in hardware/DAC; no physical audio-age claim. |
| Controller capture → RTC | Latest-value coalescing; pulse FIFO expires100ms | Latest snapshot prevents replay; pulse FIFO has no explicit count bound. UI-generated pulses are rate-limited by interaction, but no arbitrary-producer memory proof.50ms refresh is not sender acknowledgement. |
| RTC control/frame report → SCTP | Immediate transport-capacity admission; latest feedback, old feedback>250ms rejected | Zero retransmits alone does not prevent unsent queues; existing admission protects this. Remote game receipt/processing still not measured. |
| UI commands / RTC events | 16 /32 bounded channels | Try-send and coalescing avoid hot-path blocking; progress can still be lost/delayed on full mailboxes. Shutdown waits on worker/transport completion. |
| Mic capture → RTC voice | Three clips,80ms expiry; mute epoch fence | 16kHz capture/20ms Opus,48kHz RTP clock. Old/muted clips rejected including final-send check. Hardware capture scheduling and acoustic echo are device-dependent. |
| Catalog image work → cache/UI | Four foreground/four prefetch tasks, bounded results8, HTTP timeout8s | Image result send may block its own worker. No shared AVC lock; queued HTTP/image decode can consume resources during stream; device contention unmeasured. |
| Diagnostics producers → disk export | Trace4096, incidents1024, history2100 one-second snapshots | Bounded memory ring; old detail overwritten. Memory locks/formatting and periodic status logging cost time even with overlay hidden. Disk dump occurs on exit; no measured Vita overhead comparison in this audit. |

### Negotiation, scheduling and preserved features

Home and Cloud use distinct service endpoints/start payloads and stream kind.
Requested1280×720, server-reported3840×2160 and decoded960×544 output are not
interchangeable. Recovered SPS proves1280×720 Main/level32, refs1,reorder0,dpb1
for those recordings. The decoder reserves1280×720 input and renders960×544.
The offer's profile/limits and server response alone do not prove future payload
properties. In particular advertised H264 baseline/level limits do not establish
that a server always sends baseline. Unexpected reference/profile properties
remain a compatibility risk; do not interpret zero errors as validation.

RTC receive/control runs independently (Vita CPU1/time-critical); AVC worker on
CPU2; rendering/UI and audio playback on app thread. Hardware calls occur
outside the short surface ownership lock. Pressure-debt polling services delayed
outputs; fake hardware tests cover starvation and refused poll behavior. No
explicit audio master clock makes video wait for audio. They can independently
be late; correcting local audio age does not synchronize physical A/V.

Mic encoding/capture/mute/levels are retained. Chat renegotiation now refreshes
video feedback policy from the actually applied answer. A declined voice
direction does not skip this if the answer was installed. Existing native Opus
and two-peer chat tests pass. No new echo canceller was added; the user's absence
of echo is preserved as an observation, not guaranteed by a host test.

Bottom-left mic, bottom-center Guide and bottom-right quick settings share the
translucent style. Guide is absent from quick-settings entries; diagnostics
toggle remains. Touch ownership prevents overlay taps leaking into game controls.
Rear swap maps L2↔L3 and R2↔R3 only on rear touch; front zones/physical mappings
unchanged. A source test exists for this mapping but is not included in the
SDL-free host harness: live controller/touch verification remains pending.
Account collections use server history/order, pagination and deduplication;
favorites are explicitly unsupported by the current provider, not a newly
implemented cross-device favorite service. Image/auth activities were not
redesigned. Auth token storage's remove-then-write update is not crash-atomic;
this is a separate durability risk, not evidence for the streaming incident.

## Evidence-backed issue register and executed plan

Priority order: eliminate session-destructive cleanup; preserve media identity/
age; prevent stale feedback control; validate dependencies/overload; package a
traceable candidate. No optional catalog/UI redesign is part of remediation.

| ID / status | Defect, evidence and consequence | Implemented change / expected result | Regression risk / verification |
| --- | --- | --- | --- |
| L1 confirmed source defect; corrected | Login spawned an unowned all-active-session stop sweep; startup also stopped all sessions. An async sweep could delete a newly started or other-device session. Generic stop issued Home DELETE from cancellation/error paths. | Remove sweep and unowned stop helper; store StreamKind in each Stream; Home stop detaches locally without REST DELETE, Cloud stops only its owned path. Manual Home refresh stays in-session media repair. | Home attachment may persist until service timeout, and immediate reattach behavior is unknown. Production Stream/request-sink tests:100 Home stops issue no requests; Cloud DELETE only its own path. Wiring guards cover callers. Physical game preservation still pending. |
| A1 confirmed source/test mechanism; corrected | Audio age was lost at SampleBuilder/batch→Opus→PCM transitions. Eight-buffer blocking PCM send stalled decoder while older packets persisted upstream; small SDL queue did not establish freshness. | Carry first-admission Instant through every handoff, reject expired PCM, enforce existing240ms total local age budget, never block decoder on full PCM sink, join/disconnect old worker before restart/drop. | Starvation/drop can be audible under real overload; sender-age and hardware audio remain unknown. Native Opus + recording SDL sink covers6s stale packet/PCM, full sink and100 stop/start cycles; actual SDL/DAC not exercised. |
| V1 confirmed red→green regression; corrected | Input-bearing firmware call could return an unmatched older picture; code borrowed the submitted input's epoch. Output-only calls already rejected it. | Require returned-PTS match for every output; discard unidentified picture, record unknown_picture_pts. No reset or IDR storm added. | Firmware with incorrect PTS may now suppress video instead of displaying untraceable pixels. Fake hardware unknown-PTS test failed before patch and passes; real firmware identity still needs device confirmation. |
| F1 confirmed red→green controller defect; corrected | REMB fallback treated fixed≥100ms arrival offset as repeated new congestion evidence, cutting2Mbps→500kbps and pinning quality. This is not proof that bitrate caused the reported delay. | Use successive lower-envelope growth rather than absolute offset; stable paths can probe up, idle time gives no health credit. Preserve reported original offset, cap/floor and active-TWCC ownership. | Controller sensitivity/sender response remain uncertain; very slow growth below tolerance is not bounded by the model.30virtual-minute fixed-offset/skew test failed on baseline (500kbps), passes (2Mbps,zero cuts); growing bottleneck model still passes unchanged. |
| F2 confirmed configuration defect; corrected | Initial SDP refreshed feedback; later microphone answer changed peer negotiation without refreshing cached video feedback/NACK policy. | After an answer is applied, recalculate policy from current remote description, including when audio send direction is declined. | Real Xbox SDP may differ; existing two-peer voice/SRTCP tests plus source-wiring guard pass. No claim remote Xbox obeys REMB/TWCC based on counters. |
| P1 verification gap reduced | SFO00.00 and filenames cannot identify installed test build. | Verify embedded full SHA/build, clean workflow checkout, pinned prior SDK digest and hashes in machine manifest. | String presence is provenance, not executable correctness or installed-byte verification. Package and device status must agree. |
| U1 unresolved primary incident | User5–6s action-response delay; old histories show both long recovery and late relative arrival with empty local queues. | No unsupported attribution or periodic restart workaround. Preserve existing detailed diagnostics and classify their limits. | Distinguish sender/path/kernel vs local by synchronized input/console/Vita observation plus current build history. If local age rises, examine matched decoder/audio/receive-service timeline; if only arrival offset rises, capture ingress/sender timing before changing local queues again. |
| U2 significant unverified edge risks | SSRC changes with discontinuous RTP clocks, empty padding within FU sequence accounting, unsupported H264 properties, socket buffers, firmware/DAC queues and logging/resource contention. | No speculative production changes made for unobserved variants. Existing wrap/reorder tests do not cover every SSRC/padding combination. | Dedicated SSRC replacement/padding fixtures and device traffic/firmware evidence needed. These remain open, not silently declared safe. |

Competing explanations and discriminating evidence:

- Local decode accumulation: supported by some historical high decoder windows;
  contradicted as the *sole* explanation by +1991ms arrival with queue0 and136ms
  local decode. Distinguish with original-AU/returned-PTS timing during the same
  delayed interaction, not a startup FPS count.
- Input backlog: an earlier dependency test reproduces it; current host admission
  remains fresh across outages and local admission snapshots are short. These
  contradict an app-queue-only theory, not unknown console-side processing.
  Simultaneous console response and handheld response distinguish input vs video.
- Audio-only accumulation: source permits it independently of video, now age-
  bounded locally. Physical audiovisual markers are needed to distinguish sender
  delay/DAC delay from PCM queuing. Video must not wait for audio as a workaround.
- Network/sender adaptation: plausible due to arrival growth/loss and bitrate
  floor; not confirmed by extender/guest network or raw counters. A low-latency
  ingress timestamp with late media would implicate earlier stages; old datagrams
  in the kernel would implicate service/backpressure. REMB transmission alone
  cannot establish sender compliance.
- Power/save or idle-scene transitions: user observed some association and better
  Home play with Wi-Fi power save unchecked. Continued active-play/Cloud failures
  contradict declaring that setting the complete root cause.

## Automated verification completed before packaging

`tools/run_host_audit.py --output <directory>` captures commands, wall durations,
exit codes and raw logs. Host Rust1.98.1, native installed libopus; SDL output is
a controlled recording sink. Full run passes **187 Rust test executions** across
standalone feedback and ten harnesses (some shared-module tests repeat), plus
**26 Python tests**. Rust counts:15+14+9+45+4+24+39+22+1+8+6.

- Actual SCTP dependency:30virtual minutes,44 bidirectional2s outages; delivered
  report max age5ms and final-window max5ms, outstanding maximum5418B in this
  5ms one-way link model. Wall test group18.97s. Outstanding acknowledged/flight
  bytes are not an unsent-age metric. Model is not Xbox input processing.
- Actual reorder/assembly:108,000 access units (30virtual minutes at60fps),
  eight-frame bursts, out-of-order FU ends, 16-bit sequence and32-bit RTP wrap;
  byte-for-byte expected output, zero damaged-frame submissions/drops. Separate
  tests cover genuinely missing fragments, late repair, NACK, malformed/SPS
  fixtures and dependency gating. Small fixtures are not hardware-decodable
  recordings and are not claimed as faithful replay.
- Actual AVC worker with mocked firmware:39 tests cover backpressure, stalled
  calls, refusal of output-only polling, epoch cutover, unknown output identity,
  shutdown leases, bounded byte reservations and output debt. The trace-paced
  scheduling comparison uses explicit assumed service costs, not Vita emulation.
- Audio:8 tests including gain/age helpers, native Opus prediction with full
  sink, expired encoded/decoded work and100 reset/drop cycles. Test group~0.067s;
  a6s timestamp injection is a deterministic age test, not six seconds of device
  playback. Original RTP age also verified through actual SampleBuilder wrap.
- Session:6 tests including production Home/Cloud stop requests and related
  parsing. No Xbox service contacted; server timeout/lifecycle semantics unproven.
- Features: server ordering/pagination, mute epoch/clip age and two-peer voice
  negotiation/native codec covered. No acoustic/noecho or physical touch test.
- Python:16 timing-analysis tests,6 ELF-layout tests,4 source/provenance contract
  tests. Source-string guards protect wiring but are explicitly not runtime tests.
- `git diff --check` passes. Review checked dual-channel disconnect before audio
  join, metrics update ordering, surface lease exclusion, matched output epochs,
  captured StreamKind in all credential branches, Cloud owned cleanup, and no
  new periodic restart/encoded-frame age-drop path.
- Clippy completed on all ten host harnesses with exit0; warnings remain (unused
  harness imports/dead code, existing argument counts/collapsible branches,
  test formatting and audio channel tuple complexity). This is not a warning-free
  or sanitizer result. The combined test/static run took40.988s wall time; it is
  not a40-second device test or a30-minute real-time soak.

No device runtime, real SDL speaker playback, physical scanout, actual Xbox
feedback compliance, hardware leak/sanitizer run or30-minute physical play has
been performed. A successful native compile is recorded only as compilation.
Static-analysis output and final package identity are attached to the release
record after execution; they must not be inferred from this pre-build document.

## One consolidated device check (only after the host/native checks)

Use the single candidate, unchanged normal settings/network. Confirm its build
and full revision in diagnostics match its manifest. Do not repeat the already
established guest-network/power-setting experiments.

1. Home30min, then Cloud30min: ordinary active play. At~2min and~27min record a
   few discrete input/visible/audio events; include both console display and
   Vita in the Home recording if available. This distinguishes input from video
   delay; no new camera alone can establish remote Cloud capture time.
2. During each run briefly use voice/mute and diagnostics on/off. In Home, once
   pause the game without opening GreenVita menus, resume, then invoke Home
   refresh once. Same game must remain running; do not accept a restarted game.
3. Exit normally after each run so automatic history/incident/trace files are
   saved; retain Home files before Cloud can overwrite them. No manual profiling
   setup or repeated competing builds is requested. If severe lag occurs, note
   the elapsed time and exit normally then; do not force30min of unplayable use.

History automatically retains35min of one-second snapshots, incident ring and
last~4096 trace events. Export on normal exit collects originals; a forced power
off can lose them. These cannot provide a full-session per-frame p99/max or
physical E2E by themselves. Report snapshot distributions as such; annotate
camera samples and acquisition limits. Passing this check would support device
acceptance for that network/game, not universal conditions. Failure keeps U1
open and determines which clock/stage needs the next targeted capture; it is
not a reason to choose another arbitrary queue threshold.

Rollback: retain recovered RX38.18 VPK and its SHA-256 above. Reinstall that
package under the same title ID, without deleting account/config data. This
reverts app code, not console game state. RX38.18 is a known previous package,
not a claimed latency-good baseline.
