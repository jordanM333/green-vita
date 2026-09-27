# Final-build result — NOT READY

## Current C05 connection correction

The C04 shared HTTP client imposed a five-second response cutoff inside the previous ten-second total budget. A real delayed-header test fails on C04 and passes after restoring the original budget. Device-code request failures now return Back to authentication, and sanitized errors identify discovery/session/header/body stages. See [C05_CONNECTION_RESULTS.md](C05_CONNECTION_RESULTS.md) for exact changes and execution evidence. All 238 Rust test executions, 27 Python tests and 11 strict host lint suites pass. Native checking and optimized VPK packaging/integrity pass; strict native lint still fails. Current artifact: `GreenVita-C05-20260926-CONNECTION.vpk`, SHA-256 `e160daac300e3696234fabdb9c6938c7d0095ecb719b1aa80686076be1f2ad31`. These are connection-regression corrections, not a demonstrated Vita latency fix. **NOT READY**, native environment **NOT EQUIVALENT**, actual Vita test duration **zero**.

## Previous C04 delivery (historical)

The user authorized the exact pinned SDK and requested the VPK. Current native all-target/all-feature checking and optimized VPK packaging **pass**; package CRC, ELF headroom, embedded revision/build number, SELF/SFO/static assets and unchanged build inputs pass. Formatting and all 11 strict host-Clippy suites pass, with 235 Rust test executions and 27 Python tests. **Strict native Clippy fails** on unused production members, and release compilation emits warnings. Native AVCDEC safety/cleanup and sustained device latency remain unverified: **NOT READY**, native environment **NOT EQUIVALENT**, device duration **zero**. See [C04_PACKAGE_RESULTS.md](C04_PACKAGE_RESULTS.md) for exact current commands, timings, package hash and limitations. The current VPK is `GreenVita-C04-20260926-VALIDATION.vpk`; the older package recorded below is historical only.

## Earlier continuation hold (historical; superseded by C04 above)

Current source: **implemented and host-tested; native verification NOT EQUIVALENT; NOT READY**. See `SDK_VALIDATION_HOLD.md`. Formatting passes; all 11 host strict-Clippy suites pass; 235 Rust test executions and 27 Python tests pass in 28.800 seconds. Worker-state grouping, the boxed Streaming state and test-fixture corrections are recorded before edits as C01–C03 in the audit. All prior test assertions and streaming policy values are retained. Native compilation/lint/packaging have **not** run on this updated source; the previous VPK below does **not** contain these changes. Device duration remains zero. SDK artifact identity exactly matches the prior candidate, but publisher-signature validation/execution authorization are not established. Restoration completed outside the worktree before its stop signal arrived; no SDK executable ran. Further SDK/native work is paused for authorization. Full continuation commands and output are in the saved evidence archive, not committed build artifacts.

### Continuation changelog and files

No new critical/high functional fix is claimed. Medium release-gate corrections: `src/streaming/video/worker.rs` groups existing owned decoder state without changing call order; `src/api/streaming/rtc/worker.rs` groups owned RTC channels; `src/app/state.rs` and `src/app/stream_session/connection.rs` box/extract the large stream state without changing session ownership. `src/api/streaming/rtc/{feedback,mod}.rs` share one bandwidth definition; `session.rs` removes an unreachable enum wildcard. Native-only corrections are pending native compilation.

Low, behavior-equivalent cleanup: `src/api/streaming/rtc/rtp.rs`, `src/streaming/audio_gain.rs`, `src/api_xbox/streaming/control/input.rs`, `src/input.rs` collapse conditions/fixed chunk iteration or move unchanged tests. Tests: `tests/{rtc-reports,rtc-transport,frame-signal,voice-codec}/src/lib.rs` compile fixture crates only as tests, retain all tests, and add real host capture teardown/timing coverage; `src/api/streaming/rtc/reports.rs` tests numeric-only diagnostic summaries; `tests/rtc-transport/src/{delivery,webrtc}.rs` equivalent conditions plus mic status assertion; `tests/rtp-order/src/lib.rs` removes no-op defaults; `tests/decoder-pump/src/lib.rs` documents fake ABI safety and `burst_replay.rs` receives formatting only. No new warning allowance was added. Audit/results/checklist/SDK hold/README documentation separate current host evidence from historical native results and pending approval.

## Previous pass (historical; not current-binary verification)

Release hardening is implemented and host-tested. The target compiles and packages, but this is **not a production-approved build** and is not a demonstrated latency fix. No push, commit, PR, remote workflow, release change or device installation was performed. Existing project files and user data were preserved.

## Findings and severity-grouped changelog

| Severity | Implemented correction | Evidence / limits |
|---|---|---|
| Critical | Surface destruction revokes output targets and waits for decoder leases before freeing CDRAM (F01). | Real Rust lease/pump tests with fake native calls; native GXM/AVCDEC teardown remains untested. |
| High | Check decoder configuration, output pointer/stride/capacity, CDRAM arithmetic and AU lengths before native writes; initialize allocated storage (F02). | Undersized-output regression fails before and passes after; 42 decoder-pump tests pass. Exact vendor write/retention contract remains open. |
| High | Atomic temporary-write/flush/sync/replace for settings, encrypted credentials and cache; validate bounded settings and preserve a corrupt/unwritable original (F03/F04). | Actual host filesystem tests with injected write/rename failure; Vita durability not established. |
| High | Stream-count metadata/image downloads, reject excessive dimensions/pixels before image decode, bound cache admissions, validate credential destinations and remove raw response/token/session data from affected errors (F05/F06). | 22 hardening tests pass, including HTTP/socket cancellation, malformed bodies/images, image-bomb header, bounds, origin validation and cache traversal. This is not proof that all application logging is private. |
| High | Explicit task cancellation/joining, provisioning-result cleanup, quit/logout handling, bounded catalog shutdown; neutral controller state while absent/backgrounded (F07/F08). | 30 catalog shutdown cycles pass across idle, in-flight and full-result conditions; job and stop-policy tests pass. Real console/native teardown and SDL focus behavior pending. |
| High | Bound consecutive RTC pump errors with backoff and close on terminal failure (F15). | Budget exhaustion/reset tests pass. Four waits total 1.5 s, fifth consecutive error terminates; no claim that media delay is solved. |
| Medium | Validate device-code expiry/polling and Safe Memory ranges; preserve unknown key records; fail closed on insecure migration and report failed logout erasure (F09/F10). | Injected storage failure tests pass. Device AppUtil/encrypted migration/sign-out execution pending. |
| Medium | Recover poisoned microphone state by disabling capture and invalidating queued voice/tickets (F16). | Actual sender-callback panic fails before correction and passes after. Codec, RTP, mute serialization and normal audio levels unchanged. Host unwind recovery cannot recover a native panic-abort. |
| Medium | Pin the previously compiling SDK digest/compiler, use locked target builds, add fail-fast SDK checks and strict PR gates; include project/font notices (F11/F12). | Native check/package and byte-identity checks pass. Strict lint and provenance qualifications below prevent approval. |
| Low | Preserve attribution, document the actual Vita-only runtime/build/data paths, add localized storage-error fallback, and apply baseline-required rustfmt separately where possible. | Final root formatting check passes. Comprehensive localization/accessibility remains incomplete (F13). |

Complete per-file reasons are in [FINAL_BUILD_FILES.md](FINAL_BUILD_FILES.md). Pre-change decisions are in [FINAL_BUILD_AUDIT.md](FINAL_BUILD_AUDIT.md); the resource map is [CANCELLATION_OWNERSHIP.md](CANCELLATION_OWNERSHIP.md).

## Executed verification

| Final gate | Actual result |
|---|---|
| `cargo fmt --all -- --check` | Exit 0, 0.286 s |
| Host runner with strict Clippy | **230 Rust test executions and 27 Python tests pass**; runner exit 1, 41.946 s because nine Clippy suites fail |
| `make native-check` in recovered pinned SDK | Exit 0, 3.330 s; compiles all targets/features including target test code; warnings remain |
| `make native-clippy` in recovered pinned SDK | **Exit 2**, 8.749 s; 20 errors reported for each application/test target |
| `make vpk` on final source | Exit 0, 90.089 s total; optimized compilation reports 1m19s; **9 warnings**, then successful ELF/VELF/SELF/SFO/VPK packaging |
| `tools/check_vita_elf.py` | Exit 0, 0.055 s; metadata headroom 85,464 bytes, required minimum 65,536 |
| Package/source checks | VPK CRC, embedded dirty-source/build identity, SELF/SFO bytes and all packaged license notice bytes match; build-input hashes unchanged |
| Device tests | **0 minutes Home, 0 minutes Cloud**; no physical input-to-display, audio playback, echo, native teardown or sustained playback measurements |

See [verification/final-build/COMMAND_RESULTS.md](verification/final-build/COMMAND_RESULTS.md) for **every baseline and final subcommand, exit status and duration**, with retained raw logs. Generic baseline `cargo check/test/clippy/build --release` failed without the specialized Vita SDK/build-std environment; these are not relabeled as passes. An earlier specialized `cargo build --locked --release -Zbuild-std=std,panic_abort` succeeded in 336.249 s before the final microphone change; the final-source release evidence is the successful VPK build above.

Tests cover RTP reorder/loss/malformed fragments/wrap, decoder backpressure and output leases, PCM capacity/cleanup, microphone admission/mute, real host libopus roundtrip, settings corruption/write failures, HTTP body limits/timeout/cancellation and catalog ownership. Existing synthetic 30-virtual-minute transport/assembly/model tests remain synthetic; they are neither 30 minutes of device operation nor faithful replay of the supplied timing-only traces. Shared modules execute in multiple harnesses; 230 is an execution count, not 230 unique behaviors. Root target test binaries, sanitizers/Miri against the Vita ABI, SDL/device focus and actual Xbox failure/cancellation scenarios were not executed.

Before/after evidence includes the undersized native output regression and microphone lock-poison regression. The new cache-filename traversal test initially failed (19/20) and passed after sanitization (20/20); later additions bring the hardening suite to 22. The initial timeout fixture incorrectly blocked its Tokio executor while joining a server thread; its failure was retained and the fixture corrected. That fixture failure is not presented as a reproduced production defect. No test, assertion or lint threshold was weakened to obtain a pass.

## Why the delay incident remains open

The user observed working voice without echo, good startup on .9/.10, and later progressive/unplayable Home delay, with Cloud delay also reported. These remain observations, not evidence that a particular buffer or network setting caused all failures. The prior source/build mapping and competing explanations are retained in `investigation/FULL-AUDIT.md` and `investigation/RX3821-AUDIT.md`.

Existing records show .9/.10 preceded negotiated TWCC/NACK changes; later builds changed feedback, empty-packet handling, clock tracking and lifecycle policies. Their successful builds and bounded synthetic tests did not establish Xbox sender compliance, native decoder freshness or physical input-to-display latency. No matched .9/.10/current payload capture identifies one startup-regression cause. This pass leaves the established codec, bitrate/adaptation and media-queue policy unchanged.

For example, the prior retained recording reports a 2,014 ms relative video-arrival offset at 194.515 s while the AU queue is empty and local receive-to-GPU is about 103/220 ms average/max. The same recording also has a 19.155 s old-picture tail after packet silence. These observations contradict attributing all delay solely to the application AU queue, and also prevent assuming an exclusively network cause. Relative RTP arrival cannot distinguish sender/path/kernel buffering; receive-to-GPU does not measure physical scanout or input-to-display. Recorded one-second averages are not individual-frame latency distributions. See the prior audit for full distributions, recovery storms and the recording's short duration.

No new device latency measurements were obtained. Output validation, cancellation, storage and package changes address their own verified defects; none closes this incident. The stricter existing 30-minute/mode acceptance limits are carried forward in [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md). Do not ask the user to install this package while automated/native-contract blockers remain.

## Unresolved release blockers and required closure

| Risk | Why it remains / practical impact | Required next evidence or action |
|---|---|---|
| Strict lint and warning-free release fail | Nine host harness Clippy failures; native duplicate module, dead compatibility/helper members, unreachable match, nested branches, long parameter lists, large enum and test placement failures. SDK also warns about the required unstable NEON flag. Tests/build success does not override these gates. | Finish reviewed source/harness cleanup without blanket allowances or removing supported APIs, run the same strict commands; resolve/document compiler flag compatibility through the pinned target toolchain. Failure logs identify exact files/lines. |
| Exact AVCDEC contract incomplete | Public pinned SDK header defines structs/pixel format but does not fully prove pointer alignment, write extent, retention or handle thread affinity. A checked application-side buffer contract cannot prove undocumented native behavior. | Obtain authoritative native contract or instrumented Vita evidence for write bounds/stride/pixel layout/lifetimes; validate decoder/surface destruction and transfer on device. Treat possible memory unsafety as release-critical. |
| Native cleanup integration incomplete | SDK calls can block; output buffers cannot safely be freed while a native call is active. Accepted session POST with lost response has no returned ID to clean up; server TTL is unknown. | Execute startup/error/cancel/disconnect/shutdown/reconnect matrix with task/socket/handle counters and confirm server policy for lost provisioning responses. Home no-DELETE policy tests alone do not prove the console game stays alive. |
| Latency unresolved | No current synchronized sender/ingress/input/display/DAC capture and no accessible Vita/Xbox rig. Old traces omit payloads and cannot faithfully replay decode. | After earlier blockers pass, one 30-minute Home and Cloud matrix with original frozen gates, p50/p95/p99/max/trends, queue ages and recovery frequency; stop early on clear failure and revise the causal explanation. |
| Security/privacy/storage completeness | Affected network logs were redacted and credentials remain encrypted, but broad exception/privacy/localization review is not proven complete. Device Safe Memory and filesystem failures/power interruption untested. | Native credential migration/logout/storage tests plus final emitted-log/UI review. Do not claim successful erasure when filesystem/key deletion fails. |
| SDK provenance/reproducibility qualifications | Nine layer hashes checked, but original raw manifest bytes and publisher signature not verified; final remote recheck was blocked by an automatic approval-review service error. Build used extracted SDK tools plus documented host wrappers. | Authenticate the raw manifest/config and reproduce in the pinned container/clean CI. Do not treat version strings as attestation. |
| Release hygiene and manual features | Working tree intentionally uncommitted, no clean-checkout CI for these changes; dependency/native redistribution review and complete localization/controller focus checks outstanding. | Review patch, verify package notices/dependency obligations, run clean CI and retained UI/voice/control smoke tests before approval. |

This pass did not box application states, remove unused compatibility methods or restructure worker argument ownership just to silence lint; those remaining gate failures are explicit, not narrowly justified exceptions or green checks. A final release is not approved until they are resolved along with safety/device blockers.

## Build identity and rollback

- Repository/branch: `jordanM333/green-vita`, `latency-root-cause`.
- Base commit: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6`. Source is **dirty**, not a new committed revision.
- Embedded revision: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6-dirty-2b189cc806cd`.
- Build number: `final-audit-20260926`; local build, **no remote workflow identity**.
- Build-input manifest SHA-256: `2b189cc806cdc8a95ac63e07d92168889a2e2a8e5c87503256ebf2928ecdeb61`.
- Validation-only VPK: `GreenVita-final-audit-NOT-READY.vpk`, 14,867,308 bytes; SHA-256 `6e2fe83f7076e6549f6453d19c7fae47b4e38903eca235df54aeb537c5c14d63`.
- Package ID/name retained: `GRNVTEST1` / `GreenVita RX Test`; SFO version remains `00.00`. The embedded revision/build string, not that unchanged SFO number, identifies this package.
- Prior retained candidate: RX38.21, source `49baec007d3b3de20d27b6ca0671e183e000c9c5`, workflow `36217851432`, VPK SHA-256 `bbc81a9fd47859c41b479c0b38e51472782232aca9c1ce39fa0a4ecb76472518`. This is a rollback artifact, **not a known latency-good release**.

The evidence bundle contains the exact full patch, changed-file inventory, source input hashes, command logs, provenance and one validation-only package. Retain runtime-data backup and prior VPK; if rollback is later necessary, reinstall the retained same-ID package without deleting settings/account data or issuing console stop/power actions. Do not hard-reset this working tree; preserve the patch and existing work. Installation, publishing and remote changes were not performed.

**Recommendation: NOT READY.** Useful hardening is implemented with automated evidence. Production safety, strict gates and the reported user experience are not yet demonstrated.
