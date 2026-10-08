# HA10: microphone input level, input port choice, fading bottom buttons

The user's report on HA09: performance is "really nice and stable". With wired
EarPods plugged in, game audio plays through them but the microphone does not
pick up. Requests: a way to see whether the mic hears anything, a way to keep the
built-in mic while audio goes to the EarPods, and bottom buttons (mic, Xbox, quick
settings) that fade after 10 seconds and come back on a tap.

HA10 is a hypothesis until a physical Vita confirms it. Cloud sessions have
still not been captured with the latency folder. Rollback source is
204ea08 (HA09-25).

## What the code could and could not do

* **The app never chose the microphone.** Capture opens the Vita's voice
  audio-in port (`sceAudioInOpenPort(VOICE, 512, 16 kHz)`), and the Vita picks
  the built-in or headset microphone itself.
* **The SDK has no device selection.** The bindings in vitasdk-sys 0.3.3 expose
  only `sceAudioInOpenPort`, `Input`, `ReleasePort`, `GetAdopt` and `GetStatus`,
  with two port types, VOICE and RAW. Nothing selects "built-in mic, headset
  output". HA10 therefore cannot promise the built-in mic with a headset
  plugged in. It adds the one lever that exists, the RAW port, and measures
  what each port delivers.
* **A level meter existed only in the quick menu**, with no record in the
  captures.

## Changes

| Change | Where |
| --- | --- |
| Quick menu entry "Mic input: Standard / Raw (test)". Raw opens the RAW port, trying 512 and 256 samples at 16 kHz, then 256 and 512 at 48 kHz (averaged 3:1 to the same 16 kHz encoder input). If every raw format is refused, capture falls back to the voice port and says so. The choice is saved and applies at once to a live mic, without unmuting it. Standard (the default) is unchanged from HA09. | paused_overlay.rs, settings.rs, microphone.rs, microphone_capture.rs, connection.rs |
| Input-level logging. The status line, which the latency capture records once per second, gives the opened port and format, `sceAudioInGetAdopt` and the system mute status read at open, the highest peak since the mic was turned on, and how many 20 ms frames were quiet (peak under 0.2%). The open is also logged to stderr. | microphone.rs, microphone_capture.rs |
| A live level bar inside the mic button while the mic is on. The quick-menu meter stays. | screens/streaming.rs |
| The three bottom buttons stay for 10 s after the stream starts, the quick menu closes, or the front screen is touched, then fade out over 1 s. Any front touch shows them again. A tap where a hidden button sits only reveals the buttons: it does not press the button and does not reach the game. A tap elsewhere still reaches the game as before. | mic_button.rs, shell/mod.rs, stream_session/session.rs, screens/streaming.rs |
| Build identity HA10. | diagnostic.rs, final-build.yml, build_provenance.py |

## Reading the mic status on hardware

| Status shows | Meaning |
| --- | --- |
| `quiet f/f frames`, `level max 0%` | The port delivered digital silence: the Vita routed input to a microphone that sends nothing (for example a headset mic it does not support). |
| Quiet count well below the frame count, level max over a few percent | The mic hears sound; if the Xbox party does not, the problem is after capture. |
| `mute:1` | The system reports audio input muted. |
| `raw port refused (...)` | The Vita rejected every raw format; capture used the voice port. |

If both ports give silence with the EarPods, the Vita is selecting their mic
and receiving nothing from it. That cannot be fixed in software through this
API. Headphones without a microphone (3-pole plug) or a headphone-only adapter
should leave the Vita on its built-in mic, but this is unverified.

## Call-outs: constrained settings that HA10 changes

* **Audio policy (microphone only):** a new optional raw input port. The default
  voice port, its format, the Opus encoder, the 80 ms clip age and the 3-clip
  queue are unchanged. Playback audio is untouched.

Unchanged: bitrate and the fixed 3 Mbps REMB, the SDP ceilings and the
`maxBitrateKbps` 2000 hint, resolution, receive budgets, thread priorities, the
DISPLAY01 and latency captures, and Home sign-in handling.

## Verification (host only)

* New tests:
  * a tap on a hidden button is consumed for the whole gesture, the next tap
    presses it, and a tap elsewhere reaches the game (mic_button);
  * the button opacity holds for 10 s and reaches 0 at 11 s (mic_button);
  * the production streaming overlay draws the buttons at 9 s and part of the
    fade at 10.5 s, draws nothing at 11 s, and draws them again after a touch
    (presentation, SDL software rendering);
  * the status reports the opened port, peak and quiet frames, and resets when
    the mic is turned on (microphone);
  * changing the port ends the open capture's ticket, keeps the mute state, and
    drops audio from the old port (microphone);
  * an audit contract for the shell routing and the raw-port fallback.
* The Vita-only capture code was linted with Clippy on the host against a stub
  with the exact vitasdk-sys 0.3.3 signatures. It compiled with no warnings. The
  real Vita build happens in CI.
* `tools/run_host_audit.py --clippy` on Rust 1.98.1 (the CI toolchain)
  passes all 30 steps: 375 tests (HA09: 366), with strict Clippy on every
  harness. `cargo fmt --all -- --check` is clean.

## Remaining limits

* No physical Vita has run HA10.
* Whether the RAW port is accepted, and which microphone it reads with a
  headset plugged in, is unknown until it runs on hardware.
* `sceAudioInGetAdopt` and `sceAudioInGetStatus` are new calls in this app. They
  come from the same SceAudioIn stub library as the calls already in use.
