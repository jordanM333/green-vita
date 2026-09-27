# Changed files and reasons

## C05 delta from the delivered C04 source

The five production files changed for C05 are `src/http.rs` (restore established request deadline), `src/api_xbox/api.rs` (safe operation/stage context), `src/app/entry.rs` (correct sign-in failure Back destination), `src/app/stream_session/connection.rs` (context outside existing Cloud fallback), and `src/app/ui/screens/error.rs` (remove automatic host disclosure). `tests/release-hardening/src/tests.rs` adds three real HTTP/socket regression tests. C05 documentation changes are listed in `C05_CONNECTION_RESULTS.md`; the source patch preserves all earlier work below. No SDK or generated package is a repository source file.

## Continuation additions

The table below is the original pass inventory. C01–C03 structural/test corrections are detailed in `FINAL_BUILD_RESULTS.md` and recorded before edits in `FINAL_BUILD_AUDIT.md`; they supersede any earlier formatting-only classification for those files. C04 changes only the test-only temporary-directory allocator in `src/fs_utils.rs` (exclusive bounded creation instead of reusing an old fixture), plus this inventory, `README.md`, `FINAL_BUILD_AUDIT.md`, `FINAL_BUILD_RESULTS.md`, `RELEASE_CHECKLIST.md`, `SDK_VALIDATION_HOLD.md` and new `C04_PACKAGE_RESULTS.md` to record authorization, commands and current package identity. C04 adds no production stream-policy change. External preflight/adapters and generated evidence remain outside the source patch.

## Original pass inventory

Formatting-only classification compares each old/current Rust file after normalization with the same rustfmt. It does not reclassify functional changes as formatting. The complete patch is authoritative; earlier functional/formatting checkpoints are retained separately in the bundle.

| File | Reason |
|---|---|
| `.github/workflows/final-build.yml` | F11: same pinned native SDK, locked build and strict reusable/PR gates |
| `.github/workflows/latency-candidate.yml` | F11: same pinned native SDK, locked build and strict reusable/PR gates |
| `.github/workflows/release.yml` | F11: same pinned native SDK, locked build and strict reusable/PR gates |
| `.gitignore` | Exclude runtime credentials/settings/logs/local env and generated build artifacts |
| `CANCELLATION_OWNERSHIP.md` | Audit, ownership, release criteria, command results or change inventory documentation |
| `FINAL_BUILD_AUDIT.md` | Audit, ownership, release criteria, command results or change inventory documentation |
| `FINAL_BUILD_FILES.md` | Audit, ownership, release criteria, command results or change inventory documentation |
| `FINAL_BUILD_RESULTS.md` | Audit, ownership, release criteria, command results or change inventory documentation |
| `Makefile` | F11: pinned-SDK preflight, locked package/update builds and target/host gates |
| `README.md` | Actual runtime/SDK/build/data/privacy instructions, preserved credits and unresolved limits |
| `RELEASE_CHECKLIST.md` | Audit, ownership, release criteria, command results or change inventory documentation |
| `src/api/catalog/cache.rs` | F03/F06: bounded cache admission/read, safe names and atomic write |
| `src/api/catalog/worker.rs` | F06/F07: cancellable joined worker, bounded image requests/decode and safe errors |
| `src/api/streaming/mod.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/arrival_feedback.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/clock.rs` | F11: equivalent checked clock-report arithmetic cleanup |
| `src/api/streaming/rtc/congestion.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/failure_budget.rs` | F15: deterministic bounded pump retry budget and unit tests |
| `src/api/streaming/rtc/feedback.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/ice.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/latest_input.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/media.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/mod.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/peer.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/reorder.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/reports.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/rtp.rs` | F02: validate SPS/header dimensions before decode; no codec policy change |
| `src/api/streaming/rtc/session.rs` | F05: sanitize ICE/transport status; preserve routing and feedback |
| `src/api/streaming/rtc/traffic.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api/streaming/rtc/transport.rs` | F05: omit candidate/address payloads from errors |
| `src/api/streaming/rtc/worker.rs` | F07/F15: pulse bound, terminal error budget/close and native safety notes |
| `src/api_xbox/api.rs` | F05/F06: bounded API responses and validated credential destinations |
| `src/api_xbox/auth.rs` | F05/F09/F10: safe token storage/migration/logout, bounded OAuth timing, redacted responses |
| `src/api_xbox/catalog.rs` | F06: bounded catalog HTTP responses |
| `src/api_xbox/chat_sdp.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/collection_order.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/game_catalog.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/mod.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/session_kind.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/stream.rs` | F05: redacted SDP/protocol diagnostics; retain Home stop guard |
| `src/api_xbox/streaming/backend.rs` | F07: abort/await signaling tasks and join transport on stop |
| `src/api_xbox/streaming/control/channel.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/streaming/control/input.rs` | F11: equivalent warning cleanup in input encoding |
| `src/api_xbox/streaming/control/mod.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/streaming/rtc/microphone.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/streaming/rtc/peer.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/streaming/rtc/protocol.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/api_xbox/streaming/rtc/worker.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/app/command.rs` | F07: route disconnect through shared owned-session cleanup |
| `src/app/entry.rs` | F07: cleanup on exit and handle credential failure safely |
| `src/app/mod.rs` | F07: cancel replaced read-only state tasks |
| `src/app/service.rs` | F05/F07: propagate logout storage errors and clear account state |
| `src/app/state.rs` | F07: own/await cancellation, preserve created session for cleanup and show storage errors |
| `src/app/stream_session/connection.rs` | F07: cleanup failed/cancelled provisioning before leaving state |
| `src/app/stream_session/playback.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/app/stream_session/session.rs` | F07: await cleanup and return stop errors; retain Home refresh behavior |
| `src/app/titles.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/app/ui/header.rs` | F05/F07: handle logout errors and rebuild account/catalog ownership |
| `src/app/ui/mod.rs` | F04: localized non-destructive settings-save error banner |
| `src/app/ui/screens/connecting.rs` | F07: retain/drain provisioning job during cancellation |
| `src/app/ui/screens/paused_overlay.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/app/ui/screens/settings.rs` | F04: validate persistence and report failure |
| `src/app/ui/screens/streaming.rs` | F11: explicit native UI numeric types; retain overlay layout/controls |
| `src/app/ui/screens/title_list.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/catalog_preferences.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/fs_utils.rs` | F03: atomic replacement, bounded read, write/rename failure tests |
| `src/http.rs` | F05/F06: bounded streamed responses, timeouts, MIME/image validation and safe errors |
| `src/i18n/en-US.ftl` | Localized settings/credential storage failure messages with existing fallback |
| `src/input.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/jobs.rs` | F07: abort-and-await helper and cancellation tests |
| `src/main.rs` | F03/F06: wire dedicated atomic filesystem/HTTP/limits modules |
| `src/resource_limits.rs` | F06: named configurable-with-hard-max resource limits |
| `src/safe_memory.rs` | F10: checked app-owned offsets/lengths and adjacent FFI invariants |
| `src/settings.rs` | F04: bounded validated load/save, preserve corrupted original and regression tests |
| `src/shell/mod.rs` | F07/F08: actual Quit cleanup, controller loss/focus neutralization, release unused CDRAM |
| `src/shell/surface.rs` | F01/F02: detach output leases before Drop, safe target access and FFI notes |
| `src/streaming/audio.rs` | F02/F07: pre-FFI PCM capacity validation and matching real/stub SDL setup |
| `src/streaming/audio_gain.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/audio_timing.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/mic_button.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/microphone.rs` | F16: poisoned admission state fails closed; callback-unwind regression |
| `src/streaming/microphone_capture.rs` | F10: adjacent initialized sample/port/thread safety invariants |
| `src/streaming/mod.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/buffer_contract.rs` | F02: checked dimensions, RGB565 stride/capacity and allocation arithmetic |
| `src/streaming/video/catch_up.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/decoder.rs` | F02: checked output/config before native call; validate metadata; qualify affinity |
| `src/streaming/video/frame_signal.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/freshness.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/memory.rs` | F02/F07: checked allocation, full initialization, early-exit reserved memory release |
| `src/streaming/video/metrics.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/mod.rs` | F01/F02: buffer contract module and robust lease detachment/wakeup |
| `src/streaming/video/policy.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/startup.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/timing.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/trace.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `src/streaming/video/worker.rs` | F02/F10/F11: output type checks, SDK safety notes, equivalent warning cleanup |
| `src/streaming/voice_encoder.rs` | F10: document existing libopus FFI preconditions without codec parameter changes |
| `static/licenses/ATTRIBUTION.md` | F12: verbatim project/font notices and upstream credits in VPK |
| `static/licenses/GreenVita-MPL-2.0.txt` | F12: verbatim project/font notices and upstream credits in VPK |
| `static/licenses/fonts-LICENSE-DejaVu.txt` | F12: verbatim project/font notices and upstream credits in VPK |
| `static/licenses/fonts-LICENSE-Noto-CJK.txt` | F12: verbatim project/font notices and upstream credits in VPK |
| `static/licenses/fonts-LICENSE-Noto.txt` | F12: verbatim project/font notices and upstream credits in VPK |
| `tests/audio-pipeline/src/cases.rs` | F02/F07: capacity and repeated native substitute cleanup regressions |
| `tests/audio-pipeline/src/lib.rs` | F02/F07: matching testable audio/SDL initialization boundaries |
| `tests/decoder-pump/build.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/decoder-pump/src/cases.rs` | F01/F02: undersized/overflow/invalid-dimension/native error and cleanup tests |
| `tests/decoder-pump/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/feature-foundations/src/lib.rs` | F03/F04: include real filesystem/settings modules in host harness |
| `tests/frame-metadata/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/release-hardening/Cargo.lock` | F03–F10/F15: host fixtures, native substitutes and locked dependency graph for focused hardening tests |
| `tests/release-hardening/Cargo.toml` | F03–F10/F15: host fixtures, native substitutes and locked dependency graph for focused hardening tests |
| `tests/release-hardening/src/lib.rs` | F03–F10/F15: host fixtures, native substitutes and locked dependency graph for focused hardening tests |
| `tests/release-hardening/src/sdk.rs` | F03–F10/F15: host fixtures, native substitutes and locked dependency graph for focused hardening tests |
| `tests/release-hardening/src/tests.rs` | F03–F10/F15: host fixtures, native substitutes and locked dependency graph for focused hardening tests |
| `tests/rtc-reports/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/rtc-transport/src/arrival.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/rtc-transport/src/delivery.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/rtc-transport/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/rtc-transport/src/webrtc.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/rtp-order/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/session-lifecycle/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tests/voice-codec/src/lib.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `tools/check_sdk.py` | F11: fail early when required SDK/compiler/tools are missing or mismatched |
| `tools/run_host_audit.py` | Run new hardening harness and make existing Clippy gates strict |
| `tools/sdk-image.txt` | F11: immutable shared SDK image identity |
| `vendor/rtc/src/peer_connection/receive_bandwidth.rs` | Formatting only: baseline rustfmt gate failed; normalized content is identical |
| `verification/final-build/COMMAND_RESULTS.md` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/SDK_PROVENANCE.md` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/baseline/commands.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/build-identity.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/build-inputs.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/change-classification.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/commands.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/dependency-licenses.tsv` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-elf.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-format-v2.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-hardening-clippy.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-host-v2.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-host-v2/results.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-native-check-v2.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-native-clippy-v2.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-native-release.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-package.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/final-provenance.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/hardening-final-tests.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/inventory.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/mic-poison-after.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/mic-poison-before.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/native-env.sh` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
| `verification/final-build/sdk-verification.json` | Retained executed result, source/provenance, SDK or audit inventory evidence (not runtime code) |
