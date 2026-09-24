# Vita TV service device results

Recorded 2026-09-23 UTC; updated 2026-09-24 with the user's 0.2 log and iTLS inspection photos. These are user-reported Vita observations, not a developer-operated device test. Tested diagnostic: Vita TV Probe 0.2, title ID `GVTVPRB01`, source `49f32c81ca7e9df04ba7534fe8ea971b4aa579c9`, workflow `35822266268`. The log identifies the build; the browser screenshot alone does not. Version 0.3 is a native-control diagnostic correction awaiting device testing, not a service player.

## Results supplied by the user

| Service | Observed website result | Sign-in | Protected video with sound | Current verdict |
|---|---|---|---|---|
| Apple TV | User reports `C2-14900-4`, “Could not communicate using SSL.” | Not reached | Not reached | Blocked at secure connection; no playback pass |
| Hulu | User reports the same SSL error | Not reached | Not reached | Blocked at secure connection; no playback pass |
| HBO Max | Screenshot shows `https://www.hbomax.com/` and `C2-14900-4`, “Could not communicate using SSL.” | Not reached | Not reached | Blocked at secure connection; no playback pass |
| Netflix | Login page opens incompletely; user cannot select controls or log in | Cannot complete | Not reached | Blocked at unusable login page; page load only |

No protected title, sound, pause/resume, seeking, subtitles, playback session length, decoder utilization, FPS, input latency or audio mixing has been verified for any service. Embedded playback has not been executed. The user reported no bundled video-with-sound result and installed firmware **3.65**. The log's firmware API reports **3.74** with a spoof warning; this does not supersede the user's installed-firmware report. `model_api=0x00010000` is not an exact PCH model. Exact hardware model, spoof settings and clock validity remain unconfirmed. The photos show no `ur0:tai/itls.skprx` or `vs0:data/external/itls/` directory; current-package components appear absent, but this does not conclusively exclude every historical browser/certificate modification.

## Native control log and 0.3 correction

The supplied excerpt runs from process time 442 ms through 97964 ms (about 97.5 seconds of log coverage, **not a playback session**). AVPlayer/APPUTIL module loading and AppUtil initialization return zero. The 28 `CONTROL init` values have their high bit set and resemble successive allocated pointers, from `0x81227d60` to `0x83aa9640`. There are no source-open, frame, audio or shutdown records. All four service browser-launch calls return zero; that only indicates a successful handoff request, not that the website or a title loaded.

Source review identified a definite signed-handle bug: the probe treated `SceAvPlayerHandle` as a signed status code and rejected every supplied value. The same check prevented shutdown from closing those handles on retries, explaining the repeated allocation pattern; exact leaked memory was not measured. Version 0.3 replaces all player validity checks and the sentinel with opaque nonzero/zero handling, and adds audio-thread creation/start status records. Other negative status and thread-ID checks are retained. It does not change the codec, browser route, firmware or system plugins.

Host regression tests compile the actual production start/stop/control/video-polling functions against API doubles. They cover all 28 reported handles, one positive handle, null initialization, source/thread failures, pause/resume, seeking, polling, repeated restart and idempotent close. These are logic tests, not Vita decoder/GPU/audio/concurrency tests. Device playback in 0.3, visible pattern, audible tone, controls, stability and performance remain unverified. The next bounded test is the local clip for two minutes if stable, followed by one close/reopen and a new log; no protected-service inference follows from it.

## Interpretation and remaining checks

The three SSL failures establish a failure communicating securely; the browser dialog alone does not isolate protocol/cipher compatibility, certificate trust/expiry, device clock, or network interference. Netflix's partial rendering does not isolate unsupported JavaScript from failed HTTPS subresources. None of these observations is a DRM-license failure: the tests stopped before that stage.

[iTLS-Enso's own README](https://github.com/SKGleba/iTLS-Enso/blob/master/README.md) describes TLS 1.2 and updated CA certificates, and says its browser patch can be used without Enso. Its full package also changes app-side TLS, so a broad installation is not an appropriate blind change when preserving working Xbox streaming matters. The installer source exposes separate browser compatibility and CA-certificate actions. This is a possible repair for secure page access, not verified compatibility with these four websites or their protected players. No TLS patch was installed, no certificate verification was disabled, and no insecure proxy was used in this investigation.

For the separate browser investigation, check the device date/time and confirm iTLS components before changing system modules. Any applicable repair must preserve certificate verification and account for app-side networking effects. Then repeat each website independently, recording the stage reached. Stop describing the route as viable unless a protected episode/movie actually plays with sound. A new VPK that hands off the same URLs would not address the demonstrated browser failures; 0.3 addresses only the independently evidenced native-control bug.

Browser repairs also do not provide a licensed content-decryption module or service-approved device integration. [Google's Widevine overview](https://developers.google.com/widevine/drm/overview) lists Netflix, Hulu and HBO Max among users and documents licensing/device integration. Its broad “Sony PlayStation” platform entry does not establish Vita browser support. [Netflix's current browser requirements](https://help.netflix.com/en/node/30081) and [HBO Max's supported devices](https://help.hbomax.com/us/Answer/Detail/000002506) do not list Vita browser playback. [Apple's web viewing instructions](https://support.apple.com/guide/tvplus/watch-in-a-web-browser-apdc0cb7ad64/web) do not prove that this Vita can execute its player. No service-authorized replacement DRM path has been found in the inspected GreenVita project or VitaSDK interfaces.

## Delivery decision

All four services remain blocked on the tested system-browser path. There are **zero qualifying standalone service apps**, **zero working GreenVita TV integrations**, and no protected-service PiP result. Only an isolated 0.3 diagnostic correction is being built; no working service VPK is claimed. Working GreenVita/RX Test data and Xbox code remain untouched. The practical legitimate viewing fallback is each service's official app on a supported device, such as the user's iPhone; this is not Vita playback and does not fulfill the requested Vita outcome.

Reopen implementation only when a service has a credible authorized playback route to test. Success with a TLS connection, login page, synthetic clip, another service or desktop browser must not be recorded as protected playback on Vita.
