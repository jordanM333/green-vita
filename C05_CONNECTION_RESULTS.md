# C05 connection correction — NOT READY for production

This candidate corrects an introduced HTTP deadline regression and a sign-in error navigation defect. It is **implemented and host-tested**, not proof that every connection failure on the user's Vita is resolved. It does not change media latency policy or establish a streaming-latency improvement. Actual Vita duration: **0 minutes Home, 0 minutes Cloud**.

## Finding and causal evidence

The user reports sign-in, console discovery and stream-start failures on the delivered C04 build. The screenshot shows `network request timed out` at stream start, plus raw regional service hosts. It does not show elapsed request time or distinguish DNS, TLS, response-header, body, or service-side delay.

The restored C04 source exactly matches its retained binary-input map, SHA-256 `2e4a18d05b7e1f1606e4f6dfdcc33d46238bcb4674c86d2821d2e3728d0020ce`. Inspection of the locked reqwest 0.12.28 implementation shows its read timer starts before response headers. C04 introduced five-second connect/read limits below the previous ten-second overall budget in the shared client used by authentication, discovery and provisioning. This rejects responses that the previous code accepted.

A real local TCP server delayed headers six seconds. The pre-hardening reference client accepted its response; the actual C04 production HTTP client failed with `network request timed out`. The same regression test passes after restoring phase limits to the existing ten-second total budget. This is an HTTP behavior reproduction, not a physical Vita/Xbox failure reproduction; the fixture paths do not execute actual Microsoft authentication or console APIs. If the device fails outside this timing window, another cause remains to be established.

## Changelog and exact C05 files

| Severity | File | Change and reason |
|---|---|---|
| High | `src/http.rs` | Restore the previous ten-second request allowance consistently across connect/read/overall timers. The total deadline still includes headers and body and never resets after headers. Distinguish connection failure from request/body failure without exposing URLs. |
| Medium | `src/app/entry.rs` | Device-code acquisition failure uses the existing sign-in error state, so Back returns to authentication initialization instead of unauthenticated mode selection. Verified by state-flow inspection and native compilation; device navigation pending. |
| Medium | `src/api_xbox/api.rs` | Add constant console-discovery, response-header and response-body contexts to sanitized errors. Do not wrap or change the known Cloud offering fallback code. |
| Medium | `src/app/stream_session/connection.rs` | Add session-creation context outside `api.start_stream`, after its Cloud offering fallback has run. |
| Low | `src/app/ui/screens/error.rs` | Remove unconditional regional host display from errors. Existing settings display is unchanged; this is not a global endpoint-redaction claim. |
| Validation | `tests/release-hardening/src/tests.rs` | Add actual delayed-header, shared total-deadline and cancellation-before-headers socket tests. Retain existing tests and assertions. |
| Documentation | `FINAL_BUILD_AUDIT.md`, `README.md`, `FINAL_BUILD_RESULTS.md`, `FINAL_BUILD_FILES.md`, `RELEASE_CHECKLIST.md`, `C05_CONNECTION_RESULTS.md` | Record decisions before source edits, actual results, changed files and unresolved release gates. |

No new critical fix is claimed. Exactly five production binary-input files differ from C04. Dependencies, payload limits, TLS verification, redirects, automatic retry policy, credential format, media queues, decoder, voice, controller mappings and Home session termination policy are unchanged. No commit, push, PR, remote release update or device installation occurred.

## Executed validation

Commands below ran from the existing checkout through retained host/native environment adapters. The evidence archive contains exact argv, cwd, elapsed time and full output in `verification/c05/*.json` and `*.log`; `host-all/results.json` records every host subcommand. These adapters are external build evidence, not source configuration or a bundled SDK.

| Command / gate | Exit / actual duration | Result |
|---|---|---|
| `cargo test --locked --offline --target x86_64-unknown-linux-gnu --manifest-path tests/release-hardening/Cargo.toml valid_slow_headers_keep_the_existing_ten_second_budget -- --nocapture` before fix | 101 / 0.053 s | Environment failure: pruned registry index; not regression evidence. |
| Same before-fix test without `--offline` | 101 / 26.669 s including compile; test 6.00 s | Reference succeeds, production client times out: failing-before evidence. |
| `rustup component add rustfmt clippy` | 0 / 6.133 s | Restores missing host rustfmt component; an earlier format attempt could not run. |
| `cargo fmt --all` and harness formatting | 0 / 0.200 s and 0.049 s | Formatting applied; no independent semantic change. |
| `cargo fmt --all -- --check` | 0 / 0.210 s | Pass. |
| `cargo test --locked --target x86_64-unknown-linux-gnu --manifest-path tests/release-hardening/Cargo.toml -- --nocapture` after fix | 0 / 11.362 s | All 25 focused tests pass, test execution 10 s. |
| `python3 tools/run_host_audit.py --output <evidence>/host-all --clippy` | 0 / 109.701 s | 238 Rust test executions, 27 Python tests, all 11 strict host Clippy suites pass. Shared modules may execute in multiple harnesses. |
| External `restore_sdk.py` | 0 / 187.853 s | Exact authorized manifest, config and nine compressed-layer SHA-256 hashes validated before extraction. |
| External SDK preflight | 0 / 0.882 s | Compiler/linker, target, sysroot, headers/libraries, environment and package tooling checked. |
| `make native-check` | 0 / 106.579 s | Locked target all-target/all-feature check passes, with existing warnings. Target test code compiled; not executed. |
| `make native-clippy` | 2 / 6.788 s | Fails: seven production dead-code error groups, six on the test target; required unstable NEON flag also warns. No allowances or weaker gate added. |
| External `package_current.py` → `make vpk` | 0 / 216.990 s | Optimized compile 3m23s, eight application warnings, successful ELF/VELF/SELF/SFO/VPK packaging. |
| External `verify_package.py` | 0 / 0.607 s | ELF metadata headroom 90,140 bytes; CRC, embedded identity, SELF/SFO/static assets and unchanged build inputs pass. |

The delayed-body test receives headers after six seconds, then stalls; the original total deadline still terminates the request between nine and twelve seconds and closes its socket. The cancellation test aborts and joins the real HTTP task before headers, confirms socket closure and no replacement connection. These cover bounded host request behavior, not server-side cleanup after a session POST whose response is lost. Existing body-limit, settings, microphone teardown, decoder lease, timestamp, transport and Home stop-policy tests also pass.

## Build identity and package

Base commit: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6`, branch `latency-root-cause`, plus preserved uncommitted C04 changes and this C05 delta. Build number: `C05-20260926`. Embedded revision: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6-dirty-b2d09414f878`. No remote workflow was run. The retained wrapper commands above are this build's execution identity.

- Artifact: `GreenVita-C05-20260926-CONNECTION.vpk`, 14,868,364 bytes.
- SHA-256: `e160daac300e3696234fabdb9c6938c7d0095ecb719b1aa80686076be1f2ad31`.
- Binary-input map SHA-256: `b2d09414f8787028ea896aeaa10764ae5e5ac6bfdcc25616db67898ebfee08dd` (sorted compact JSON files map; includes uncommitted source).
- Package title remains `GreenVita RX Test`, title ID `GRNVTEST1`. The SFO generic version remains `00.00`; use the embedded C05 build/revision to identify this artifact. This is homebrew packaging, not Sony signing/certification.
- The separate review ZIP preserves the full patch against the base, a compact C04→C05 functional diff, old evidence and current command/provenance records. SDK bytes and build caches are excluded. Compilation alone does not approve release.

SDK: community `vita-rust/vitasdk-rs`, source `https://github.com/vita-rust/docker`, immutable reference `ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc`. OCI source revision `c58598173d9e6de9adce6b07443519143e7c7d82`, config SHA-256 `e1fc73fb0bc96ed3f2a607e3872a8ad08a69f0965e7fec5597d2c1d99539ee47`. Rust `1.97.0-nightly (4b0c9d76a 2026-05-10)`, target `armv7-sony-vita-newlibeabihf`, GCC 15.2.0, cargo-vita 0.2.2. SDK bytes **EXACT MATCH** the previously authorized pin. No publisher signature was verified. Tools ran from an external extracted tree with host adapters; native environment remains **NOT EQUIVALENT** to hermetic container CI. SDK files/tools/caches are excluded from the source patch and deliverables.

Rollback: retain the previously delivered C04 VPK and runtime-data backup. C04 SHA-256 is `777cbb50a3aa2c1f63cc49081d9992486952e1cfc8a21d2af81d3958d8e920ea`. No settings or account migration is introduced; rollback does not require deleting credentials or stopping the running Xbox game. C04 is not claimed to be connection- or latency-good.

## Remaining boundary and decisive device check

**NOT READY** remains the production recommendation: strict native lint fails; full AVCDEC ownership/write-contract and native cleanup gates remain open; there are no hardware interaction-delay/recovery measurements. The frozen 500 ms maximum interaction delay, one-second recovery and 30-minute-per-mode criteria remain unchanged. Successful host HTTP tests and VPK packaging cannot close those gates.

For this correction, use the existing account and network, open Home, list the consoles and start the same game. If sign-in fails, Back should return to the sign-in flow. A successful sequence establishes that these device operations work on C05; it does not establish sustained streaming quality. If it fails, the error now identifies console discovery or stream-session creation and header/body stage where applicable. Retain that error text and build number; do not erase settings or supply account credentials. There is no new telemetry upload or automatic request replay.
