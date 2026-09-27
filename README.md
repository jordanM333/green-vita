# GreenVita

GreenVita is a Rust homebrew client for Xbox Cloud Gaming and Xbox Home Remote Play on PS Vita. It derives from [Day-OS/green-vita](https://github.com/Day-OS/green-vita). This checkout is a release-hardening candidate, **not an approved final release**. See [FINAL_BUILD_AUDIT.md](FINAL_BUILD_AUDIT.md) and [RELEASE_CHECKLIST.md](RELEASE_CHECKLIST.md).

The sole application runtime target is `armv7-sony-vita-newlibeabihf`. Linux and Windows are build hosts; this repository does not implement a desktop player. A homebrew-enabled Vita, network access, Microsoft/Xbox account and the applicable Xbox service/console permissions are required. Cloud availability is determined by Microsoft. GreenVita is not affiliated with Microsoft or Sony.

Home and Cloud use separate session/signalling paths. Supported features include controller/rear-touch mappings, microphone mute and voice chat, audio-level controls, bottom mic/Xbox/quick-settings controls, and a diagnostics toggle. Cloud collections use service-provided history/order where available. Favorites are not claimed to be account-synchronized without a verified service contract; legacy local preferences remain readable.

**Known release blockers:** sustained device latency, complete native buffer-contract evidence and device cleanup validation remain open. A bounded local queue or successful build does not establish input-to-display latency. No hardening change in this pass is advertised as a latency fix.

## Reproducible Vita build

Use the immutable SDK image in [`tools/sdk-image.txt`](tools/sdk-image.txt):

```text
ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc
```

The pinned digest matches the retained RX38.21 native CI record. The continuation validated the original registry manifest, config and all nine compressed layer hashes; no publisher signature was verified. The user subsequently authorized this exact SDK. Target checking passes after full external preflight; strict native Clippy still fails. Extracted-tool execution remains NOT EQUIVALENT to hermetic container verification. See `C05_CONNECTION_RESULTS.md` for current connection-regression results, `C04_PACKAGE_RESULTS.md` for the previous delivery and `SDK_VALIDATION_HOLD.md` for the earlier hold and exact metadata. Local tool/version checks are not cryptographic attestation.

This image supplies VitaSDK, SDL2, Opus, AVCDEC/GXM headers/libraries, cargo-vita, and Rust `1.97.0-nightly (4b0c9d76a 2026-05-10)` with rust-src. The exact tested compiler is pinned through the image, rather than asserting an untested minimum Rust version. Cargo.lock preserves registry and git dependency revisions; no broad dependency upgrade is required.

From a Linux/macOS shell with Docker installed:

```sh
SDK_IMAGE=$(cat tools/sdk-image.txt)
docker run --rm -v "$PWD:/work" -w /work "$SDK_IMAGE" make native-check
docker run --rm -v "$PWD:/work" -w /work "$SDK_IMAGE" make native-clippy
docker run --rm -v "$PWD:/work" -w /work "$SDK_IMAGE" make vpk
```

On Windows PowerShell with Docker Desktop using Linux containers:

```powershell
$SdkImage = (Get-Content tools/sdk-image.txt).Trim()
docker run --rm -v "${PWD}:/work" -w /work $SdkImage make vpk
```

Inside that SDK, `make sdk-check` fails early for missing tools or a mismatched compiler. `make vpk` produces `target/armv7-sony-vita-newlibeabihf/release/green-vita.vpk`. `make eboot` builds the executable. For a development build use `cargo build --locked -Zbuild-std=std,panic_abort` inside the same SDK. Do not assume generic host `cargo build` or `cargo run` builds/runs a Vita application. Native tests can be compiled here but require a Vita runner to execute; the host harnesses below cover testable Rust logic with explicitly documented native substitutes.

Install a locally built VPK through VitaShell. Existing `make upload-vpk VITA_IP=...` and `make update-run-vita VITA_IP=...` targets require an explicitly chosen device and network access. Treat the device address as private. Do not automatically deploy or publish a candidate that fails the checklist. The current test package ID remains `GRNVTEST1`; this pass does not rename the product or silently migrate runtime directories.

## Host development checks

Use Rust 1.98.1 with rustfmt/Clippy, Python 3, a C compiler and libopus development libraries. The root Cargo configuration targets Vita intentionally. Host tests specify the host target explicitly:

```sh
cargo fmt --all -- --check
python3 tools/run_host_audit.py --output verification/host --clippy
cargo test --locked --target x86_64-unknown-linux-gnu --manifest-path tests/release-hardening/Cargo.toml
```

The runner records each command, exit status and duration. It exercises production Rust modules with local HTTP fixtures, deterministic timing, real host libopus and fake SDK/SDL boundaries where stated. Synthetic timing and queue tests are not a replay of Xbox packet payloads or a measurement of Vita playback. PR CI runs host formatting/tests/strict Clippy plus target checking/strict Clippy/release packaging in the pinned SDK. A failing gate blocks release; warnings are not globally disabled.

## Sign-in, settings and privacy

Use the in-app Microsoft device-code flow. Enter the displayed code only at the validated Microsoft sign-in URL. No `.env`, embedded secret, private API key or manually copied token is required. Console discovery requires the signed-in account's Home Remote Play access. Offline devices, rejected credentials and network failures should show errors rather than require editing files.

Runtime data is under `ux0:data/green-vita-540-test/`: `settings.json`, encrypted `xcloud-tokens.json`, `cache/catalog-v1/`, and diagnostic exports. Refresh tokens use ChaCha20-Poly1305; the random key is held in AppUtil Safe Memory. Legacy plaintext-token migration fails closed if encryption fails: the app attempts removal and reports the failure. Sign-out attempts both file and key cleanup and reports inability to erase. Device Safe Memory and filesystem failure testing remains a security release gate; this is not a claim of guaranteed erasure. Never share that directory wholesale or include pairing codes, tokens, account names, console IDs, SDP or ICE addresses in bug reports.

Settings writes use temporary-file/flush/rename replacement. Invalid, oversized or unreadable settings remain on disk; in-memory defaults and a storage-error banner are used. Back up and repair that file deliberately before trying to save again. Host atomic-write tests do not establish Vita filesystem durability under power loss. Cache refusal at the count/byte budget is safe: artwork may be fetched without being saved. Cache clearing remains an explicit user action.

HTTP reads enforce named caps in `src/resource_limits.rs`, before accumulating whole responses; image dimensions/pixels and decode allocations are checked separately. Connection/read limits preserve the existing 10-second total request budget, including headers and body; individual reads do not restart that total deadline. C05 removes the premature five-second cutoffs introduced in C04. No implicit HTTP retries are enabled; credential destinations remain validated. Device-code polling uses server expiry/interval bounds and a bounded failure retry policy.

## Reporting and device acceptance

Report the embedded build/revision, Home or Cloud mode, duration before failure, whether voice/diagnostics were enabled, and only the reviewed trace/status evidence. Logs can still contain sensitive data in paths under audit; review before sharing. A useful device test measures physical input-to-visible response and audio synchronization over at least 30 minutes per mode; receive-to-GPU timing alone is insufficient. Home refresh must preserve the running game. See the checklist for the consolidated matrix and `CANCELLATION_OWNERSHIP.md` for cleanup boundaries.

## License and attribution

GreenVita remains licensed under [MPL-2.0](LICENSE). Preserve upstream Day-OS attribution and source notices when redistributing modified source. Bundled font license notices and the project license are copied into `static/licenses/` for packaging. Native/dependency redistribution review remains part of the release checklist. The decoder's existing reference-memory approach credits MattKC's Vanilla project in source. Xbox, PlayStation and related names belong to their respective owners.

## Credits

- [Greenlight](https://github.com/unknownskl/greenlight), an open-source xCloud
  and Xbox home-streaming client that served as a protocol and UX reference
- [xbox-xcloud-player](https://github.com/unknownskl/xbox-xcloud-player), the
  WebRTC streaming library used by Greenlight and a key reference for xCloud
  and xHome session handling
- [Vita Moonlight](https://github.com/xyzz/vita-moonlight), a major reference
  for low-latency streaming and hardware video decoding on the PS Vita
- PS Vita icon used in the GreenVita logo: "PS Vita" by Mark Davis from
  [The Noun Project](https://thenounproject.com/icon/ps-vita-203775/)
- [VitaSDK](https://vitasdk.org/) and the
  [vita-rust](https://github.com/vita-rust) ecosystem

GreenVita is an independent homebrew project and is not affiliated with or
endorsed by Microsoft, Xbox, Sony, or PlayStation.
