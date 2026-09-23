# RX38.14 returned recording and arrival-feedback candidate

RX38.14 is **not accepted for latency**. The user reports that it still becomes laggy and unplayable. The returned four files identify revision afc20bbd73537d50108ca489a7ab8792cf65bbb3. Raw recordings remain outside the public repository. Candidate RX38.15 source is 165f42963e1fe6d1754a6e836fd3ba06c1bc7e5a; CANDIDATE-38.15-BUILD.json records its identity and verification result.

## What this recording establishes

The history covers approximately 427.6 seconds. The rolling incident and detailed trace buffers cover only the retained tail, not the entire session.

| Elapsed | Observed behavior |
| --- | --- |
| Up to about 180 s | Approximately 60 decoded/shown fps, 2–5 ms added video arrival delay, about 66–69 ms local receive-to-GPU average. |
| 182.264 s | Incoming UDP 5,873 kbps and video payload 4,320 kbps, despite a 2,000 kbps requested ceiling. 290 of 298 receive passes reach their budget. |
| 186.275 s | Added video arrival delay 371 ms; local receive-to-GPU average 99 ms. ICE round trip 119 ms. |
| 190.288 s | Video arrival growth 1,388 ms; local receive-to-GPU average 98 ms. 60 decoded / 52 shown fps. |
| 243.363 s | Maximum observed video arrival growth 1,837 ms. |
| 255.066 s | Manual video refresh; recovery completes about 112 ms later on the same session. The subsequent delay buildup returns. |
| Roughly 300–360 s | Low video payload; 60/60 fps and near-zero arrival growth return. |
| 410.592 s | Video arrival growth 813 ms, audio 173 ms; compressed queue reaches 24 frames and local receive-to-GPU average 128 ms. |
| 419–421 s | Video arrival grows to around 1.5 s; a recovery waits 1,597 ms. |
| 427.632 s | UDP 836 kbps versus video payload 39 kbps; all 220 receive passes reach the budget. Video/audio arrival growth 148/156 ms; ICE round trip 156 ms. |

The dominant added delay during the first sustained episode precedes local decode/display. Later episodes also include local compressed-queue pressure, recovery waits and increased presentation cost. These are interacting costs, not evidence that a single render setting will clear the problem. Receive-budget hits demonstrate processing load, not the location or exact size of an OS/socket backlog.

The packet-order diagnostics finish with 1,939 holes filled, 33 released losses, and 796 queued NACK requests. Filled holes include ordinary reordering; they do not by themselves prove retransmission success. Frame integrity improvements have not prevented sustained delay. Video payload repeatedly exceeds the requested limit, so lowering the requested bitrate again is not a demonstrated fix.

Arrival growth is relative to the fastest observed RTP arrival and excludes fixed network/capture delay. Unsynchronized sender clock offsets are not treated as capture-to-photon latency. The lower final payload rate does not explain total traffic: RTP overhead, audio, padding, RTCP, DTLS and STUN were not separately counted in this build. Do not infer a particular traffic flood or blame the user's access point from that total alone.

## Next changes

1. **Negotiated packet-arrival feedback.** The previous application offers REMB/loss feedback but neither advertises nor generates transport-wide congestion feedback. The new chain uses the pinned RTC library's TWCC receiver at its default 100 ms reporting interval. It offers the standard transport-cc feedback and transport sequence extension for audio/video. Reports use authenticated RTP arrival timestamps and go through SRTCP. No new media jitter buffer is introduced; unsupported peers retain existing behavior. The remote sender still decides whether/how to react.
2. **First-packet codec binding.** In the vendored RTC endpoint, an SDP-declared SSRC whose codec is identified on the first RTP packet did not notify interceptors. The RID and undeclared-stream paths did. That missing call prevented negotiated extension metadata from reaching the feedback receiver. The fix supplies codec and negotiated header-extension information at discovery. The first bootstrap packet can precede that binding; subsequent packets are observed. TWCC wraps the existing receiver-report layer so repeated bindings do not reset loss history.
3. **Traffic attribution.** Per-window diagnostics classify incoming/outgoing datagrams as RTP, RTCP, DTLS, STUN request/indication, STUN response, or other. Incoming classes include packet counts, kbps, total processing milliseconds and maximum per-packet processing microseconds. Classification inspects clear headers for diagnostics only; it does not bypass RTC authentication or reject traffic. Separate counters show TWCC-bound tracks, observed extension packets, successfully transmitted TWCC datagrams and authenticated empty RTP payloads.

The microphone topology, capture/Opus path, volume controls, three translucent buttons, session-preserving Refresh, REMB thresholds and bounded NACK/recovery policy are retained.

## Verification and device interpretation

All 156 host test executions passed in workflow 35825348209 (shared production tests run in multiple suites). New regressions negotiate real peers through ICE/DTLS/SRTP/SRTCP and inspect **decrypted** arrival feedback at the sender. A declined-extension case verifies continued media and receiver reports without TWCC. A deterministic interceptor test covers negotiated metadata arriving after the clock fallback, repeated binding without loss-history reset, transport sequence wraparound, a missing packet, arrival deltas and unbinding. Existing microphone renegotiation, audio, controls and encrypted NACK tests now use the production arrival-feedback chain. Header classification/accounting have independent tests.

The next build is a targeted protocol correction, not a proven resolution of this device recording. Device acceptance requires playing past the prior failure window, ideally ten minutes, and comparing arrival growth, packet classes, TWCC track/packet/sent counts and visible recovery. Zero TWCC tracks/packets/sends means this session did not activate that mechanism; it must not be called an effective congestion fix in that case. Home and Cloud need separate results.

Native Vita release compilation and VPK conversion passed. The package CRC, artifact/VPK checksums and embedded source/build identity were verified; title ID is GRNVTEST1. Executable metadata headroom is 102,976 bytes against a 65,536-byte minimum. Only eboot.bin differs from RX38.14. VPK SHA-256: 73a96289a433a3d6203ca71bd9ab5e5a7eab5747d0a50fb4b4e83eb8ec8523a7. Device acceptance remains pending.
