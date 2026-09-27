# Final release checklist

Current recommendation: **NOT READY**. Compilation and host substitutes do not prove Vita safety, physical presentation, successful Xbox cleanup, or sustained latency. Check a box only against retained execution evidence; see `FINAL_BUILD_RESULTS.md` for this pass.

## Provenance and automated gates

- [ ] Reviewed, clean checkout identifies the exact committed source. No untracked source is omitted from the evidence bundle.
- [x] Cargo.lock and vendored patches retained; no broad dependency updates.
- [x] Release SDK is pinned by digest in `tools/sdk-image.txt`; local recovered identity/source documented.
- [x] `cargo fmt --all -- --check` passes on C05 source (exit 0, 0.210 s).
- [x] `python3 tools/run_host_audit.py --output <evidence>/host-all --clippy` passes every host test and strict lint gate (exit 0, 109.701 s; 238 Rust executions and 27 Python tests).
- [x] `make native-check` passes on C05 source with authorized SDK and full external preflight (exit 0, 106.579 s). Extracted environment remains NOT EQUIVALENT to hermetic container CI.
- [ ] `make native-clippy` passes without suppressed warnings.
- [ ] `make vpk` / target release build passes without unresolved warnings.
- [x] C05 package passes ELF headroom (90,140 bytes), CRC, embedded identity and SELF/SFO/static/source checks (exit 0, 0.607 s). This is not device execution.
- [ ] Native tests execute on an available Vita runner. Generic host Cargo commands are not a substitute for this SDK/runner.
- [x] Undersized decoder output, microphone poison and cache traversal tests retain failing-before/passing-after evidence; fixture failures and their corrected assumptions are retained.
- [x] Final diff reviewed for leases, races, drops, task ownership, integer arithmetic, secret leaks and unintended API/mapping/streaming-policy changes. This review identified unresolved native-contract/privacy/lint risks; it does not approve release.

## Release-critical safety and lifecycle

- [ ] Exact AVCDEC write size, pixel order, stride units, pointer alignment and retention rules established from authoritative contract or instrumented native validation. Host fake decoder tests alone cannot close this item.
- [ ] Malformed input rejected before FFI; valid RGB565 output is initialized and large enough; buffer/decoder/library destruction order tested on device.
- [ ] Missing/corrupt settings and failed writes preserve the previous file and show an actionable message. Successful persistence survives stop/start and power interruption on Vita.
- [ ] Credential migration, Safe Memory failure and sign-out failure tested on Vita. No plaintext credentials remain after a claimed successful migration/sign-out; inability to erase must remain visible.
- [ ] No reachable device, sleeping/offline device, rejected/expired authentication, pairing cancellation, malformed response, DNS failure and timeouts do not crash or loop indefinitely.
- [ ] Initial startup, load/save, discovery, sign-in, connection, clean disconnect, interruption, reconnect and app shutdown exercised.
- [ ] Ten connect/disconnect cycles per mode leave no worker, socket, decoder, audio device or stale account state behind. Record resource counts before/after, not just successful navigation.
- [ ] Cancel provisioning/SDP/ICE, disconnect during decode/audio/image download, fill catalog result queue and exit during a cache write; no unsafe early release or deadlock.
- [ ] Home manual and automatic media recovery never issue DELETE, quit-game or power-off; the running game and progress remain intact. Verify actual console behavior.
- [ ] Controller unplug/replug and focus/background transitions release input; shoulder/rear-touch swaps retain their meaning.
- [ ] Working voice/mute/audio levels, no reported echo, account/game selection, Home/Cloud, bottom-corner controls, bottom-center Xbox, quick-settings removal of duplicate Xbox and diagnostics toggle preserved.
- [ ] User-visible messages/localization fallback and keyboard/controller focus tested. Diagnostic labels are not a substitute for an accessible control.

## Sustained device matrix (not yet performed)

Run one consolidated 30-minute sequence in **each** mode on the same known-good home network. Do not mix guest-network and home-network results. Keep the console network path, game, requested quality and Vita power settings constant. The previously supplied traces remain evidence; do not ask for the same observations again.

| Minutes | Condition |
|---|---|
| 0–5 | Baseline play, voice off, diagnostics off; record startup latency as well as steady state |
| 5–10 | Voice on, diagnostics off; compare game/chat levels and echo with the working baseline |
| 10–15 | Voice on, diagnostics on; capture consistent RTP/receive/decode/display timing |
| 15–20 | Voice off, diagnostics on; pause the game and set down/pick up the Vita as previously reported |
| 20–25 | Resume active play; one Home media refresh (Cloud recovery as supported), verify the game remains running |
| 25–30 | A brief controlled network interruption followed by active play; verify recovery and audio alignment |

Use a video recording that includes physical controls and display, with at least 20 comparable visible-action samples per five-minute window. Record audio reference events where measurable. Do not derive input-to-display from receive-to-GPU timing. Report sample counts and measurement uncertainty; missing physical timing is a failed evidence gate, not a zero-latency sample.

**Carry forward the already frozen gates in `investigation/FULL-AUDIT.md` and `investigation/RX3821-AUDIT.md`:** compare minutes 2–5 with minutes 25–30. Relative-arrival and receive-to-GPU p95 must not grow by more than 50 ms; no added media delay above 250 ms sustained for five seconds; no measured action-to-visible response above 500 ms; no recurrence of the reported 5–6-second lag. These are engineering regression limits, not measured Vita specifications. Report p50/p95/p99/max, per-window distributions and trend, including startup samples. Do not substitute an empty queue or a good frame rate for these checks.

Audio must not retain old local work after a stall; retain the existing 240 ms SDL safety bound. Actual audible delay must not progressively grow and must return within 100 ms of the initial measured A/V relationship after recovery. After an isolated disturbance, playback must become current within **one second** after delivery and hardware service return to normal. Ongoing packet loss is reported separately, not counted as successful recovery. No automatic session restart, repeated refresh loop or recovery storm on an intact reference chain. Report every recovery cause/duration and all input/AU/decoder residence distributions. Home refresh must cause no REST stop/play, quit or power command and preserve the same running game.

The earlier draft of this checklist proposed weaker 1-second interaction / 3-second recovery limits. That draft was corrected during document review, before any hardware evaluation; the stricter existing limits above remain authoritative. No measured failure was excluded or reclassified.

Actual sustained device duration in this pass: **0 minutes Home, 0 minutes Cloud**. No physical input-to-display or DAC timing measurements were collected. Host virtual-time tests do not count toward these durations.

## Distribution, privacy and rollback

- [ ] Review logs/errors/crash/UI data for tokens, pairing codes, account/device IDs, SDP, endpoints and IPs; only reviewed evidence is shared.
- [x] Project MPL-2.0, upstream credits and existing bundled-font notices preserved and included in static package assets.
- [ ] Review all locked Rust/native dependency redistribution obligations and icon attribution against the actual package. A license inventory alone is not legal clearance.
- [x] C05 Vita VPK/SELF packaging and byte integrity validated. This is homebrew packaging, not Sony signing/certification; runtime safety is still an open gate.
- [ ] Retain prior installed VPK and runtime-data backup. Roll back using the retained package and backup, not by deleting user settings or changing console/game state. Document that the prior candidate was not proven latency-good.
- [ ] Obtain explicit approval before pushing, opening a PR or publishing/replacing a release. The user authorized the exact SDK and requested a remedy to the delivered build; C05 is a correction candidate, not an approved final production release. No Git/remote release mutation occurred.
