# SDK validation and authorization hold — 2026-09-26

**Authorization update:** The user subsequently approved this exact SDK and requested the VPK. Full external SDK preflight, native checking and current-source packaging now pass; strict native Clippy fails. See `C04_PACKAGE_RESULTS.md` for actual execution evidence. No signature, native-equivalence, AVCodec safety, or Vita acceptance gate is waived. Everything below records the earlier hold and is historical, not the current execution status.

**Artifact identity: EXACT MATCH. Release: NOT READY. Native verification: NOT EQUIVALENT.**

The current raw registry manifest, config descriptor, and all nine compressed layer digests match the retained SDK record. The RX38.21 CI log records this same image digest, checkout `49baec007d3b3de20d27b6ca0671e183e000c9c5` and build number `38.21`. This establishes artifact identity, not a publisher signature, permission to execute it, a successful current native build, or Vita behavior.

## Exact identity and its evidence boundary

| Field | Recorded value |
|---|---|
| Registry owner / publisher source | `vita-rust`; config source is `https://github.com/vita-rust/docker`. There is no separate vendor/publisher label or verified signing identity. This is community Vita-Rust/VitaSDK, not Sony's proprietary SDK. |
| Product | Image repository `vitasdk-rs`; OCI title `docker`; description `vitasdk/vitasdk docker image with rust toolchain` |
| Version label | `latest` — not an immutable version and not used to select the artifact |
| Source revision / build identifier | OCI revision `c58598173d9e6de9adce6b07443519143e7c7d82` |
| Build timestamps | Config `created`: `2026-05-11T05:58:18.165969113Z`; OCI created label: `2026-05-11T05:56:47.983Z` |
| Official community source | `https://github.com/vita-rust/docker`; its README explicitly names this GHCR repository. No approved internal mirror was supplied. |
| Immutable artifact reference | `ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc` |
| Required container / original manifest SHA-256 | `351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc` — recomputed from original response bytes; matches |
| Config SHA-256 | `e1fc73fb0bc96ed3f2a607e3872a8ad08a69f0965e7fec5597d2c1d99539ee47` — 4,680 bytes; recomputed and matches |
| Layer checksums | All nine compressed sizes and SHA-256 values match the hash-validated manifest and retained previous record; full list in `sdk-restoration.json` in the continuation evidence. This is content identity, not publisher authentication. |
| Signature / attestation | No signature is supplied inside the manifest/config. No external publisher signature/attestation was located or verified. Do not infer that the artifact is signed, or that no external signature exists. |
| Image runtime | Config `os=linux`, `architecture=amd64`; build history names Alpine `3.23.4` x86_64 |
| Rust toolchain | Previous executed record: `rustc 1.97.0-nightly (4b0c9d76a 2026-05-10)`, host `x86_64-unknown-linux-musl`, with rust-src. OCI history says `RUST_TOOLCHAIN=nightly`; it does not declare the resolved compiler version. Not executed in this continuation. |
| Package tooling | Previous executed record: cargo-vita `0.2.2`; version is not declared in OCI labels and is not re-executed now. |
| Application target | `armv7-sony-vita-newlibeabihf` from the repository configuration; not a claim that this triple is declared by the OCI manifest |
| Expected image environment | `VITASDK=/usr/local/vitasdk`, `RUSTUP_HOME=/usr/local/rustup`, `CARGO_HOME=/usr/local/cargo`; PATH includes cargo and VitaSDK binaries. These are container paths, not developer machine paths. |
| Previous distributed candidate | RX38.21; workflow `36217851432`; source `49baec007d3b3de20d27b6ca0671e183e000c9c5`; VPK SHA-256 `bbc81a9fd47859c41b479c0b38e51472782232aca9c1ce39fa0a4ecb76472518`; same SDK digest above |
| Previous local validation package | `final-audit-20260926`; source `8d9cba8b22b82cddfe9e81e352558132c2fcbbc6-dirty-2b189cc806cd`; same SDK digest, but extracted tools and host wrappers, not a hermetic container-equivalence proof |

Host prerequisites for the documented release route: a working Linux/amd64 container runtime (or a documented compatible VM/runtime), repository checkout with locked dependencies, network/cache access for the pinned dependencies, and a writable build volume. Within the image: nightly Rust/rust-src/build-std, arm-vita-eabi compiler/linker/sysroot, VitaSDK/SDL2/Opus/AVCDEC/GXM libraries, make, CMake, Python, Perl, pkg-config, cargo-vita and Vita ELF/SELF/SFO/VPK tools. Docker/Podman are absent here. Executing extracted tools with host wrappers is **not** automatically equivalent to that route.

## Restoration and authorization chronology

Before the user's explicit authorization hold, a metadata-and-layer retrieval process had already been started. It validated the original manifest hash before downloading or extracting any layers. It extracted each layer only after that layer's size/hash matched. When the hold arrived, inspection found eight completed layers; the ninth finished before the targeted stop signal arrived. The stop returned “No such process”; the restoration process had exited 0 after 161.045 seconds. Therefore restoration completed, rather than being successfully interrupted. **No SDK executable, native compiler, SDK preflight, native build, or package tool was run in this continuation.**

The SDK was extracted outside the Git worktree. It remains unused; it is not included in the patch or evidence archive. No SDK files, keys, credentials, caches or generated build outputs were committed. No Git commit/push/PR/release modification occurred. Further execution is paused pending explicit authorization of this exact digest or an approved alternative. Metadata-only config retrieval/hash comparison after the hold did not extract or execute tools.

## Preflight required before any native execution/build

The existing `tools/check_sdk.py` checks only some tools and the compiler version; it is not sufficient for the newly requested preflight. After authorization, extend and host-test it before execution to check:

1. Exact authorized image/metadata identity; required VITASDK/Rust/Cargo environment; no unexpected tool substitution.
2. Compiler full version/host, supported target triple and rust-src/build-std availability.
3. Resolved C compiler/linker/archiver, target identity and sysroot; required headers, startup objects and libraries.
4. cargo-vita and ELF/SELF/SFO/VPK tooling identities; output paths and locked build invocation.
5. Clear fail-fast errors and a non-secret relative-path build-input manifest. No version-string check is represented as signature attestation.

No preflight result exists for this continuation. Do not run native check/lint/test/build/package until it passes. Native tests need a Vita runner; compilation of tests does not execute them.

## Commands and actual status

Exact commands, working directories, elapsed time and raw output are retained externally in `verification/continuation/`; SDK/tool artifacts are excluded. The non-secret SDK identity JSON contains no credentials or local workspace paths.

| Operation | Actual status |
|---|---|
| Initial restore script using `requests` | Exit 1 in 0.030 s, twice: `requests` unavailable; no SDK operation occurred in those attempts |
| Standard-library restore script | Exit 0, 161.045 s; raw manifest and nine layer hashes matched; extraction completed before stop signal |
| `sha256sum` of original manifest | Exit 0; exact pinned SHA-256 above |
| Metadata-only config validation | Exit 0, 14.627 s; config length/hash matched |
| `cargo fmt --all -- --check` | Exit 0, 0.197 s |
| `python3 tools/run_host_audit.py --output <evidence>/c03-host --clippy` | Exit 0, 28.800 s; 235 Rust test executions, 27 Python tests, all 11 strict host-Clippy suites pass |
| SDK preflight / current native check / current native strict lint / native test execution / native release build / VPK packaging | **NOT RUN — authorization hold** |
| Vita hardware validation | **0 minutes Home, 0 minutes Cloud** |

Host coverage includes fake-ABI decoder shutdown/lease/stale-frame tests, 20 repeated real host microphone capture-worker start/error/drop/join cycles, real host Opus/DTLS voice tests, and distinct receive/submit/completion timing tests. It is not Vita firmware, AVCodec native contract, physical presentation or audio-playback verification.

No new VPK was built. The retained VPK predates continuation changes and must not be described as their binary. Recommendation remains **NOT READY** until strict native gates, documented/checked AVCDEC ownership and buffer/lifetime requirements, rebuilt VPK integrity/provenance, and device cleanup/latency evidence all pass. Preserve **500 ms maximum interaction delay** and **one-second recovery** criteria; no threshold was relaxed.

## Required decision

Authorize execution of the exact digest above with the explicitly unverified publisher-signature limitation, or supply the approved signed/internal artifact and verification policy. Until then, native verification is **NOT EQUIVALENT** and SDK execution remains on hold.
