# HA02-17 returned Cloud blackout evidence

Source 39f3e3cec780294e4cea5f93d6e88dd2eaea5587, build HA02-17. User still observes video becoming black while audio continues. This observation remains authoritative; submitted frames do not prove visible output.

## Findings
- First video 2.126 s; first queue-completed video draw 2.360 s. The startup trace covers through 5.125 s. At seconds 3 and 4 there are 42 completed video draws each; no retained startup live-edge quarantine event. First playback baseline establishes at 2.363 s.
- Capture tail 170.126–182.126 s also has continuing decode, selected pictures and completed draw callbacks. The final status shows 60 decoded/60 shown in its one-second window, current Live timing and no unmatched decoder PTS.
- The middle packet trace is absent; 2,821 lock skips. Status history shows five live-edge incidents and five reported recoveries by the tail. Therefore the manifest's NOT REPRODUCED label only means its separate sustained-deficit capture trigger did not fire. It does not negate the user's blackout or mean there was no delay event.
- Retained original-dequeue-to-GPU-callback times: median 70.696 ms, p95 91.101 ms, max 117.970 ms (822 records). These are client residence observations, not optical presentation.
- Production streaming overlay + production SDL painter, real egui fonts, and first captured status were rendered in a host software SDL test for 180 frames with diagnostics off and 180 with diagnostics on. 89.6% / 52.0% of background pixels remain unchanged in the final frames. No full-screen blackout reproduced. This does not emulate Vita GXM.
- Home screenshot is the explicit service rejection SigninBlockedByPasswordPrompt before connect is accepted. No media decoding begins on that path. A console sign-in prompt must be resolved on the console; no password/security workaround is implemented.

## Decision
No demonstrated streaming-policy correction follows from these counters. Disabling live-edge protection or changing thresholds would be speculative. The narrow device-only unknown is the actual pixel data across the native decode/upload/display boundary. The supplied bundle has no compressed payloads, decoded pixels, texture pixels, or OS framebuffer observations, so neither replay nor offline analysis can reconstruct them.

DISPLAY01 adds a bounded ten-second capture: submitted AUs for independent decoding; same-generation source and uploaded samples; framebuffer samples after the existing GXM wait. It makes no streaming or recovery policy changes. Native picture pointer/stride contract bits are recorded in existing telemetry. It is a diagnostic build, NOT a corrected hardware-acceptance candidate. Details/limits and procedure are in DISPLAY-PROBE.txt.

## Returned file SHA-256
- `manifest.json`: `6aabacbb3ae4643e6d5753f9d7fef8534dfce2b89bf3006470ce4921242ce4d1`
- `events.csv`: `d986f93f9fbb53565c6fc6b59a4a43fc47f4a47122c3b5f0a3af4fc38bb31ffb`
- `history.txt`: `820687db3924ec39d00eb6a5607b0a2f4ef35347f72d016b2d4ee0ecd6580560`
- `README.txt`: `7a377df5907cdd68d1ffabefd0a22a8ded6acd7ea65f9f2fbb78eed973aeaf9e`
