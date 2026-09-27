# C04 validation package — NOT READY

Date: 2026-09-26. The user authorized the exact previously reported SDK digest and requested delivery. This is a current-source, locally built **validation VPK**, not a production-approved release or demonstrated latency correction. No install, commit, push, PR or remote workflow/release mutation was performed.

## Package identity

- Filename: `GreenVita-C04-20260926-VALIDATION.vpk`; 14,864,321 bytes.
- SHA-256: `777cbb50a3aa2c1f63cc49081d9992486952e1cfc8a21d2af81d3958d8e920ea`.
- Branch: `latency-root-cause`; base commit: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6`.
- Embedded revision: `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6-dirty-2e4a18d05b7e`; build number: `C04-20260926`.
- Build-input map SHA-256: `2e4a18d05b7e1f1606e4f6dfdcc33d46238bcb4674c86d2821d2e3728d0020ce`. It hashes compact, sorted-key UTF-8 JSON mapping actual build-input paths to SHA-256. Full map is retained in `c04-build-inputs.json`; all mapped inputs remained unchanged through packaging/verification.
- SELF SHA-256: `dd502fb163df081f7dc74216fa9aa6be0037cfc8c057a68ca98d3d297d1d6290`.
- Existing install identity preserved: `GRNVTEST1`, `GreenVita RX Test`, SFO `00.00`. Use the embedded build/revision, not this unchanged SFO number, to distinguish builds.
- No CI run ID exists for C04. Prior distributed RX38.21 used workflow `36217851432` and commit `49baec007d3b3de20d27b6ca0671e183e000c9c5`.

## SDK identity and execution boundary

**EXACT MATCH**: `ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc`. Raw manifest/config and all nine compressed layer hashes validate. Config SHA-256: `e1fc73fb0bc96ed3f2a607e3872a8ad08a69f0965e7fec5597d2c1d99539ee47`. Community source/publisher: vita-rust, `https://github.com/vita-rust/docker`, OCI revision `c58598173d9e6de9adce6b07443519143e7c7d82`. No publisher signature was verified; execution proceeded with the user's explicit authorization of this disclosed identity.

Actual compiler: `rustc 1.97.0-nightly (4b0c9d76a 2026-05-10)`, host `x86_64-unknown-linux-musl`; target `armv7-sony-vita-newlibeabihf`; C compiler `arm-vita-eabi-gcc 15.2.0`; cargo-vita `0.2.2`. Full preflight checks compiler/linker/sysroot/rust-src, environment, headers/libraries, package tools and tool hashes. Its non-secret build-input manifest uses image-relative paths.

Docker/Podman are unavailable. Native execution used extracted SDK tools with external environment configuration, including SDK Python/Perl module paths and a previously absent host loader symlink pointing to the external SDK loader. A missing extracted `usr/lib/libc.so` link caused proc-macro linkage failures; its exact target `../../lib/ld-musl-x86_64.so.1` was checked in hash-validated layer `6d7cc491afccd470167d39cd3074135664a5303eb607c512ba45de75ce4cd0bd` before restoring only that absent link. Why it was absent is not determined. An empty external target directory avoided reusing affected generated proc macros. Old files were preserved. These are build-environment corrections, not streaming fixes. **Native environment verification remains NOT EQUIVALENT to hermetic pinned-container CI.** SDK tools/layers/caches are not committed or distributed.

## Commands actually executed

The evidence archive retains exact argv, cwd, exit and duration in JSON and complete stdout/stderr logs under `verification/continuation/`. `native-env.sh` is the external SDK adapter, not a repository source file. All native commands first pass the external full preflight; Makefile also runs its narrower SDK check.

| Record / command | Exit | Seconds | Actual result |
|---|---:|---:|---|
| `c04-host`: `python3 tools/run_host_audit.py --output <evidence>/c04-host --clippy` | 1 | 29.141 | One filesystem test fixture collided with an old directory before its production assertion |
| `c04-host-v2`: same command, output `c04-host-v2` | 0 | 30.284 | 235 Rust test executions, 27 Python tests, all 11 strict host-Clippy suites pass |
| `c04-format`: `cargo fmt --all -- --check` | 0 | 0.273 | Pass |
| `restore-sdk-libc-link`: external hash-validated link restoration script | 0 | 12.546 | Restores only verified absent SDK link |
| `authorized-preflight-v5`: `native-env.sh python3 <evidence>/preflight.py` | 0 | 1.016 | Full checks pass; SDK identity and tool hashes recorded |
| `authorized-native-check-v5`: `native-env.sh make native-check` | 0 | 72.224 | `cargo check --locked -Zbuild-std=std,panic_abort --all-targets --all-features` passes with warnings |
| `c04-native-clippy`: `native-env.sh make native-clippy` | 2 | 7.434 | Strict `cargo clippy --locked -Zbuild-std=std,panic_abort --all-targets --all-features -- -D warnings` fails |
| `c04-package`: `python3 <evidence>/package_current.py` | 0 | 215.443 | Sets exact embedded identity, runs `native-env.sh make vpk`; `cargo vita build vpk --release --locked` compiles optimized target then packages ELF/VELF/SELF/SFO/VPK |
| `c04-package-verification`: `python3 <evidence>/verify_package.py` | 0 | 0.545 | ELF headroom 89,328 bytes (minimum 65,536), embedded identity, CRC, SELF/SFO/static bytes and source input hashes pass |

Strict native Clippy reports seven production unused-code groups: `VideoRtp::receive`; `ApiClient::{get_active_session_paths,delete_json}`; `collect_session_paths`; `Stream::session_id`; `StreamingSession::video_frame`; `CatalogPreferences::{toggle_favorite,record_played}`; `DecodedFrame::{texture_index,generation}`. The test target reports six groups. Release compilation reports eight warnings (one duplicate), including the unstable NEON target-feature warning. No allowance or fake call was introduced to hide these. Compilation success does not satisfy warning-free/strict-lint acceptance.

Earlier preflight/check failures are retained: host Python/musl loader conflict, missing Perl shared-library/module paths and rustc proc-macro SIGSEGV before the verified libc link restoration. See each `authorized-*.json/.log`; none is reclassified as a pass. Native tests were compiled by all-target checking but **not executed**. No standalone generic `cargo build --release` pass is claimed; this Vita application requires the specialized SDK/build-std route.

## Changes and limitations

This VPK includes the already audited hardening and C01–C03 changes; it is not the older `final-audit-20260926` binary. The only C04 Rust edit is test-only: exclusive bounded creation of filesystem fixture directories prevents reused PID/sequence names from opening old fixtures. The failing test and complete suite then pass with all assertions preserved. No production queue, feedback, bitrate, timing or recovery threshold was changed in C04.

Release remains **NOT READY**: native strict lint is failing; exact AVCDEC alignment/write/retention/thread-affinity requirements and native cleanup remain unproven; complete privacy/storage/manual feature review remains open; extracted SDK operation is not hermetic CI equivalence. Hardware duration is **0 minutes Home, 0 minutes Cloud**. No input-to-display/audio latency distribution exists for this build. The frozen **500 ms maximum interaction delay and one-second recovery** criteria remain unchanged and untested. Host fake-ABI shutdown/stale-frame tests, real host microphone teardown/Opus tests and timestamp separation are host evidence only.

The request authorizes supplying this validation artifact; it does not make it the final production build requested earlier. Do not treat another successful compilation or package check as closing the latency incident. Keep the prior installed VPK and runtime-data backup; reinstall the retained same-ID package to roll back without deleting account/settings data or sending console stop/power commands. RX38.21 is retained historical rollback evidence, not a proven latency-good version.
