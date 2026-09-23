# Vita TV Probe 0.2

An installable **diagnostic**, not a working streaming-service app. The purpose of this first build is to gather evidence on the actual Vita before building service apps or modifying GreenVita. Apple TV, Netflix, Hulu and HBO Max protected playback are all **unverified**. Opening the browser, signing in, decoder counters and the bundled clip cannot pass a service.

## 0.2 installation correction

The 0.1 release used RGBA PNGs for LiveArea artwork. A device install returned **0x8010113D**, matching the incompatible icon/background encoding. Version 0.2 generates opaque 8-bit indexed PNGs and checks image dimensions, encoding, palette, CRCs and LiveArea references before publishing. The original 0.1 VPK fails the new encoding check. See [LiveArea image specifications](https://github.com/hammerill/livearea-specs). This corrects the identified packaging defect; successful installation and runtime playback of 0.2 still require device confirmation.

Install 0.2 through VitaShell in place of the rejected 0.1 VPK. If a Vita TV Probe bubble already exists, accept its update. The app ID and data folder are unchanged. No database rebuild, firmware change, or GreenVita reinstall is needed for this correction.

## Install and rollback

1. Copy `Vita-TV-Probe-0.2.vpk` to your Vita and install it using VitaShell. Open the new **Vita TV Probe** bubble. It needs the normal homebrew/unsafe-app permission for native system modules; it installs no kernel plugin.
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
| Local H.264 + AAC video/audio and controls | Implemented; runtime unverified |
| Apple TV protected episode/movie | Not tested on Vita; not supported by this build |
| Netflix protected episode/movie | Not tested on Vita; not supported by this build |
| Hulu protected episode/movie | Not tested on Vita; not supported by this build |
| HBO Max protected episode/movie | Not tested on Vita; not supported by this build |
| Protected titles, playback actions, session length, observed performance | None verified on a Vita; collect with instructions above |
| Service subtitles and volume | Test through the service/browser; no native service adapter |
| GreenVita TV section / Xbox coexistence / picture-in-picture | Not implemented; dependent on a proven authorized playback route |
| Overlay over other Vita games/apps | Not attempted; separate plugin/compositor, decoder and stability investigation required |

The control player logs initialization/errors, displayed-frame submissions, successful PCM-output calls, pause/seek/volume requests, elapsed time and media time. These are not decoder utilization, measured rendering FPS, input latency, or audio-mixing measurements. This build supplies no DRM, CDM, proxy, credential store, protected-media capture, or remote playback bridge. System browser cookies are shared with the system browser; this is not yet a separate service app/data design. No background telemetry or log upload exists. Firmware reported by the system API may be spoofed, and the model API does not distinguish OLED from Slim.

Known limits: the browser handoff and all native runtime behavior still require hardware testing; synthetic-clip compatibility does not establish protected-stream compatibility. The control player only handles mono/stereo PCM from AVPlayer and has fixed seek bounds for its 12-second clip. The sample has no subtitle track. There is no crash recovery or automated service certification. This build leaves Xbox source and data untouched; it does not claim a new Xbox regression test.

## Reproduce

The dedicated `.github/workflows/tv-probe.yml` only builds on the `vita-tv-probe` branch. It generates original PNG assets and the synthetic video, uses the pinned VitaSDK image, and publishes the VPK plus SHA-256/package-check metadata. Build details are also packaged in `build-info.json` and appended to the device log.

Locally, with FFmpeg, Python 3 and VitaSDK including vita2d installed:

```sh
python3 tv-probe/make_assets.py
cmake -S tv-probe -B tv-probe/build
cmake --build tv-probe/build --parallel 2
python3 tv-probe/verify_vpk.py tv-probe/build/Vita-TV-Probe-0.2.vpk
```

The source in this folder is MIT licensed. It uses VitaSDK interfaces and libvita2d; no third-party video sample or service artwork is included.
