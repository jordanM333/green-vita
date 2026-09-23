# Vita TV service device results

Recorded 2026-09-23 UTC. These are user-reported Vita observations, with an HBO Max screenshot, not a developer-operated device test. Latest delivered diagnostic: Vita TV Probe 0.2, title ID `GVTVPRB01`, source `49f32c81ca7e9df04ba7534fe8ea971b4aa579c9`. The screenshot shows the system browser; it does not independently identify the running probe version.

## Results supplied by the user

| Service | Observed website result | Sign-in | Protected video with sound | Current verdict |
|---|---|---|---|---|
| Apple TV | User reports `C2-14900-4`, “Could not communicate using SSL.” | Not reached | Not reached | Blocked at secure connection; no playback pass |
| Hulu | User reports the same SSL error | Not reached | Not reached | Blocked at secure connection; no playback pass |
| HBO Max | Screenshot shows `https://www.hbomax.com/` and `C2-14900-4`, “Could not communicate using SSL.” | Not reached | Not reached | Blocked at secure connection; no playback pass |
| Netflix | Login page opens incompletely; user cannot select controls or log in | Cannot complete | Not reached | Blocked at unusable login page; page load only |

No protected title, sound, pause/resume, seeking, subtitles, playback session length, decoder utilization, FPS, input latency or audio mixing has been verified for any service. Embedded playback has not been executed. The bundled unencrypted control clip's outcome has not been reported. Current device model, installed firmware, firmware spoof configuration, clock validity, and installed iTLS components are unconfirmed for this test. Historic device/firmware mentions are not substituted for these measurements.

## Interpretation and remaining checks

The three SSL failures establish a failure communicating securely; the browser dialog alone does not isolate protocol/cipher compatibility, certificate trust/expiry, device clock, or network interference. Netflix's partial rendering does not isolate unsupported JavaScript from failed HTTPS subresources. None of these observations is a DRM-license failure: the tests stopped before that stage.

[iTLS-Enso's own README](https://github.com/SKGleba/iTLS-Enso/blob/master/README.md) describes TLS 1.2 and updated CA certificates, and says its browser patch can be used without Enso. Its full package also changes app-side TLS, so a broad installation is not an appropriate blind change when preserving working Xbox streaming matters. The installer source exposes separate browser compatibility and CA-certificate actions. This is a possible repair for secure page access, not verified compatibility with these four websites or their protected players. No TLS patch was installed, no certificate verification was disabled, and no insecure proxy was used in this investigation.

The bounded next step is to establish the actual installed firmware and existing iTLS state, and check the device date/time. Any applicable repair must preserve certificate verification. Then repeat each website independently, recording the stage reached. Stop describing the route as viable unless a protected episode/movie actually plays with sound. A new VPK that hands off the same URLs would not address the demonstrated browser failures; no replacement wrapper build is justified by this report.

Browser repairs also do not provide a licensed content-decryption module or service-approved device integration. [Google's Widevine overview](https://developers.google.com/widevine/drm/overview) lists Netflix, Hulu and HBO Max among users and documents licensing/device integration. Its broad “Sony PlayStation” platform entry does not establish Vita browser support. [Netflix's current browser requirements](https://help.netflix.com/en/node/30081) and [HBO Max's supported devices](https://help.hbomax.com/us/Answer/Detail/000002506) do not list Vita browser playback. [Apple's web viewing instructions](https://support.apple.com/guide/tvplus/watch-in-a-web-browser-apdc0cb7ad64/web) do not prove that this Vita can execute its player. No service-authorized replacement DRM path has been found in the inspected GreenVita project or VitaSDK interfaces.

## Delivery decision

All four services remain blocked on the tested system-browser path. There are **zero qualifying standalone service apps**, **zero working GreenVita TV integrations**, and no protected-service PiP result. No new VPK or GreenVita code change is made from these observations. Working GreenVita/RX Test data and Xbox code remain untouched. The practical legitimate viewing fallback is each service's official app on a supported device, such as the user's iPhone; this is not Vita playback and does not fulfill the requested Vita outcome.

Reopen implementation only when a service has a credible authorized playback route to test. Success with a TLS connection, login page, synthetic clip, another service or desktop browser must not be recorded as protected playback on Vita.
