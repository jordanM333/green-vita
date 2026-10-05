# HA08: clean display — no build label or capture status on screen

The user's report with HA07-23: "The experience this tune was nearly
flawless. Can you button up the last changes needed including removing the
build name and capture info as the top of the display?"

HA08 changes only what is drawn on screen. Media behavior is HA07's.
Rollback source is 7c7324ab7fec3a483c840a5cdeabd39432f064c4 (HA07-23).

## The returned HA07-23 DISPLAY01 capture

* **The replay is complete.** 332 AUs (5.5 s, 0.73 MB), not truncated, with
  0 lock skips. FFmpeg decodes every AU without error.
* **The display path matches.** In all 21 pixel samples, decoder output,
  uploaded texture and OS framebuffer agree, with 0 upload mismatches.
* **The black second half is the Xbox's content.** From about 3.1 s the Xbox
  sent 40-byte frames. Independent FFmpeg decoding of them is black (mean luma
  4.7-5.9, against about 55 before), and the Vita showed the same. The
  analyzer's `INCONCLUSIVE_BLACKOUT_ORIGIN` is resolved by that decode: black
  content, not a client fault.
* The display capture does not record Home or Cloud, and the latency folder,
  which does, was not returned. So Cloud is still not confirmed by a capture.

## Changes

| Change | Where |
| --- | --- |
| The top banner no longer shows the build label or capture status. It appears only for a video problem: interrupted, recovering, waiting for the first picture, timing changed, or unavailable/delayed. | streaming.rs |
| The quick menu no longer shows "RX Test <build> · Home/Cloud streaming" or capture status. | paused_overlay.rs |
| Menu screens no longer show capture status under the header. | header.rs |
| The status-text functions these used are removed. | diagnostic.rs, display_probe.rs |
| Build identity HA08 (label in capture manifests, build number, provenance). | diagnostic.rs, final-build.yml, build_provenance.py |

Unchanged:
* all HA07 media behavior: the REMB rule, the 3 Mbps ceiling, local catch-up
  and the AU queue;
* bitrate, resolution, receive budgets, thread priorities, audio policy and
  SDP ceilings, all as in HA07;
* the DISPLAY01 and latency captures, which still record and save as before.
  The label stays in their manifest.json;
* the opt-in diagnostics overlay (quick menu), which still shows the build
  number;
* Home sign-in handling.

## Verification (host only)

* A new presentation test renders the production streaming overlay. During
  live play nothing covers the top half of the picture; an interruption still
  shows its banner. With a banner forced on during live play, the test fails.
* `tools/run_host_audit.py --clippy` on Rust 1.98.1 (the CI toolchain)
  passes all 30 steps: 412 Rust tests (HA07: 411), with strict Clippy on every
  harness. `cargo fmt --all -- --check` is clean. The native Vita check,
  clippy and packaging run in CI.

## Remaining limits

* With no on-screen capture status, the device no longer confirms that a
  capture was saved. Check the folders with VitaShell.
* Cloud has still not been captured with the latency folder, so Cloud
  behavior rests on the user's play report.
