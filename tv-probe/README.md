# Vita TV Probe 0.3

An installable **diagnostic**, not a working streaming-service app. The purpose of this first build is to gather evidence on the actual Vita before building service apps or modifying GreenVita. Apple TV, Netflix, Hulu and HBO Max protected playback are all **unverified**. Opening the browser, signing in, decoder counters and the bundled clip cannot pass a service.

## 0.3 player-handle correction

The user-supplied 0.2 log contains 28 nonzero AVPlayer handles from `0x81227d60` to `0x83aa9640`, but no `CONTROL add_source` entry. The probe incorrectly compared opaque player handles against zero as signed status codes. The high bit on these handles made the probe reject them, skip the clip, and also skip cleanup before subsequent attempts. This was a probe bug, not evidence of an unsupported video codec or DRM failure.

Version 0.3 uses a zero/null sentinel for the player throughout initialization, pause, seeking, video polling and shutdown. Real signed error checks remain on status-returning functions and thread IDs. Host regression tests compile those actual production functions against API doubles and replay all 28 logged handle values, plus null/positive cases, source/thread failures and repeated cleanup. These tests do not run the Vita decoder, GPU, real threads or speakers. The code follows the opaque-handle use in [lpp-vita's video player](https://github.com/Rinnegatamante/lpp-vita/blob/master/source/luaVideo.cpp); [Vita3K's AVPlayer implementation](https://github.com/Vita3K/Vita3K/blob/master/vita3k/modules/SceAvPlayer/SceAvPlayer.cpp) also notes the pointer ABI.

Fully close the old probe before installing 0.3 as an update. First test only the bundled clip: moving pattern plus audible tone, X pause/resume, seeking, volume, Square restart, then Circle/back and reopen once. Run for two minutes if stable. The log should now progress beyond `init_handle` to `add_source`, thread startup, frames and audio counters. A later error is a separate unverified stage; send the new session log rather than repeatedly retrying. This change does not fix browser SSL, website JavaScript or protected-service playback, and does not install iTLS or change firmware.

## Earlier 0.2 installation correction

The 0.1 release used RGBA PNGs for LiveArea artwork. A device install returned **0x8010113D**, matching the incompatible icon/background encoding. Version 0.2 introduced opaque 8-bit indexed PNGs and checks image dimensions, encoding, palette, CRCs and LiveArea references before publishing; these checks remain in 0.3. The original 0.1 VPK fails the new encoding check. See [LiveArea image specifications](https://github.com/hammerill/livearea-specs). The user's subsequent log confirms that 0.2 installed and ran, but not that its control clip played.

If a Vita TV Probe bubble already exists, accept its update. The app ID and data folder are unchanged. No database rebuild, firmware change, or GreenVita reinstall is needed for this correction.

## Install and rollback

1. Fully close the old probe, copy `Vita-TV-Probe-0.3.vpk` to your Vita and install it using VitaShell. Open the **Vita TV Probe** bubble and check the heading says **0.3**. It needs the normal homebrew/unsafe-app permission for native system modules; it installs no kernel plugin.
2. This app uses title ID `GVTVPRB01` and only writes `ux0:data/greenvita-tv-probe/probe.log`. It does not overwrite GreenVita (`GREENVITA`), RX Test (`GRNVTEST1`), their settings, or Xbox authentication.
3. To roll back, delete the **Vita TV Probe** bubble. After copying the log, optionally delete `ux0:data/greenvita-tv-probe`. Browser sign-in cookies belong to the system browser: use each service's sign-out action if desired. Deleting this diagnostic does not clear those cookies.

## First device test

- Open **Bundled video + sound control**. You should see a moving colored test pattern and hear a steady 440 Hz tone. It is an original, unencrypted 12-second clip that requests looping. Use modest system volume.
- Run it for at least two minutes. Verify X pause/resume, Left/Right seeking, L/R application volume, Square restart, and Circle returning to the menu. Record any frozen image, silent audio, desynchronization or crash. The on-screen frame/audio counters report API activity, not proof that you saw or heard output. No subtitle track is present in this control clip.
- Test each service independently from its menu. X hands its official HTTPS URL to the **system Vita browser**; this build does not embed a browser. Sign in only on the service's legitimate page. Select an actual subscription episode/movie, attempt play, confirm video AND sound, then try pause, seek, volume and subtitles if offered. Ads, trailers and login success do not count.
- Return to the probe with PS/app switching. Left/Right selects the result; Square records it. These are explicitly user-reported observations. Current-session summaries reset on relaunch; the log appends across sessions. If the browser closes the probe, reopen it and record the result then. A handoff failure is not a service playback verdict: try entering the same official URL directly in the system browser and report that separately.
- Close the app normally before copying `ux0:data/greenvita-tv-probe/probe.log` using VitaShell. Also report your exact PCH model, installed firmware (not just spoof version), exploit/homebrew setup, each title, session duration, visible error text and actions that worked. Do not send passwords, cookies, tokens or account recovery codes.

## What this build does and does not establish

| Area | Status at release |
|---|---|
| Cross-compilation, VPK integrity and separate title ID | Checked by CI; see build metadata |
| Device / installed firmware tested by developer | None; no physical Vita access |
| Previous user device evidence | 0.2 ran; user reports installed firmware 3.65, API says 3.74 (may be spoofed); exact PCH model unconfirmed |
| 0.3 device test, titles, duration and observed performance | Not yet tested; no measured performance or playback session |
| Local H.264 + AAC video/audio and controls | Implemented; runtime unverified |
| Apple TV protected episode/movie | Not reached: user reports browser SSL error; no pass |
| Netflix protected episode/movie | Not reached: user reports unusable partial login page; no pass |
| Hulu protected episode/movie | Not reached: user reports browser SSL error; no pass |
| HBO Max protected episode/movie | Not reached: user reports browser SSL error; no pass |
| Protected titles, playback actions, session length, observed performance | None verified on a Vita; collect with instructions above |
| Service subtitles and volume | Test through the service/browser; no native service adapter |
| GreenVita TV section / Xbox coexistence / picture-in-picture | Not implemented; dependent on a proven authorized playback route |
| Overlay over other Vita games/apps | Not attempted; separate plugin/compositor, decoder and stability investigation required |

The control player logs initialization/errors, displayed-frame submissions, successful PCM-output calls, pause/seek/volume requests, elapsed time and media time. These are not decoder utilization, measured rendering FPS, input latency, or audio-mixing measurements. This build supplies no DRM, CDM, proxy, credential store, protected-media capture, or remote playback bridge. System browser cookies are shared with the system browser; this is not yet a separate service app/data design. No background telemetry or log upload exists. Firmware reported by the system API may be spoofed, and the model API does not distinguish OLED from Slim.

Known limits: 0.3 native playback still requires hardware testing; synthetic-clip compatibility does not establish protected-stream compatibility. The control player only handles mono/stereo PCM from AVPlayer and has fixed seek bounds for its 12-second clip. The sample has no subtitle track. There is no crash recovery or automated service certification. This build leaves Xbox source and data untouched; it does not claim a new Xbox regression test.

## Reproduce

The dedicated `.github/workflows/tv-probe.yml` only builds on the `vita-tv-probe` branch. It generates original PNG assets and the synthetic video, uses the pinned VitaSDK image, and publishes the VPK plus SHA-256/package-check metadata. Build details are also packaged in `build-info.json` and appended to the device log.

Locally, with FFmpeg, Python 3 and VitaSDK including vita2d installed:

```sh
python3 tv-probe/tests/test_player_lifecycle.py
python3 tv-probe/make_assets.py
cmake -S tv-probe -B tv-probe/build
cmake --build tv-probe/build --parallel 2
python3 tv-probe/verify_vpk.py tv-probe/build/Vita-TV-Probe-0.3.vpk
```

The source in this folder is MIT licensed. It uses VitaSDK interfaces and libvita2d; no third-party video sample or service artwork is included.
