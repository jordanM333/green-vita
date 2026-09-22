# RX Test 38: microphone and Xbox collections

Status: published with user approval; host checks and native Vita compilation pass. The first CI attempt failed during VPK conversion; a metadata headroom fix is queued for rebuild. Device testing is still pending.

## Baseline

The user confirmed that the latest Home capture was made with **Use Wi-Fi in Power Save Mode unchecked**, and reported better gameplay. Cloud gameplay still needs comparison. This candidate retains RX37.4's video, recovery, network ordering and rendering policies. It adds an audio sending track and account collection requests; those changes still need a device regression check.

## Microphone test

1. Start Home streaming. Hold SELECT for 1.5 seconds to open GreenVita's quick menu. The microphone must start Off.
2. Join an existing game voice chat or Xbox party using the game's or Xbox's controls. This candidate does not implement a separate party browser or party creation service.
3. Select **Microphone: Off** to turn it On. Speak and check that the input meter moves and the packet count increases. Resume the game; **MIC ON** should remain visible.
4. Have a friend confirm that they hear your voice. A moving meter proves capture; the packet count only proves that the local RTC sender accepted packets. Neither proves Xbox chat routing or that a friend heard you.
5. Turn the microphone Off while speaking. Your friend should hear silence after any audio already transmitted finishes. Turn it back On and confirm there is no replay of old speech.
6. End the stream and start another. Confirm it starts Off. Backgrounding GreenVita or losing focus also mutes it; returning does not unmute it automatically.
7. Repeat steps 1–6 separately in Cloud. Compare gameplay with the microphone Off and On. Record the build number, mode, whether the input meter moves, whether the count increases, whether your friend hears you, and any displayed error.

The Vita input is 16 kHz mono signed PCM in 512-sample hardware reads, reframed into 320-sample/20 ms Opus packets at 16 kbit/s. Blocking capture and encoding run on a separate thread. RTP uses Opus's 48 kHz clock, advancing 960 ticks per packet. The shared mute gate invalidates both queued audio and an in-progress capture across mute/unmute. The queue holds at most three packets; packets older than 80 ms are discarded.

The existing audio media line now negotiates sending and receiving at session setup, while capture remains Off. The reference client instead adds its microphone track and renegotiates when requested. **Xbox acceptance of this candidate's initial negotiation is not established by the host peer test.** If Xbox does not negotiate sending, the menu reports the uplink unavailable. Audio input/encoder failures also leave it muted and show an error. A capture worker startup failure does not terminate game streaming.

No echo cancellation or input device selector is included in this first test. Use headphones for the voice test to separate your voice from the Vita's game audio.

## Catalog test

Use the collection selector at the top of the Cloud game list and **Refresh Xbox lists**.

| Collection | Source and order | Device check |
| --- | --- | --- |
| Recently played | Signed-in Xbox cloud service history, preserving server order | Play a cloud game on another device with the same account, then refresh GreenVita. The game should appear in the corresponding recent position if it is playable in GreenVita. |
| Recently added | Xbox's cloud Recently Added gallery, preserving gallery order | Compare with the website in the same market. This is catalog addition order, not game release date. |
| Most popular | Xbox's cloud Most Popular gallery, preserving gallery order | Compare the available titles with the same-market website collection. |
| Favorites | Unavailable pending a verified account service contract | Existing local bookmarks are retained for migration but are not shown as account favorites. |

Also check search within each collection, scrolling and game launch after changing collections. Cover/icon prefetch follows the displayed collection order. Returning from a Cloud session refreshes the collections; manually refresh again if the Xbox history update has not appeared yet.

Sign out and sign into another account: no previous account's recent list should appear. A failed request must show **Xbox list unavailable**, rather than local history or a misleading empty-history message. All Games remains usable while lists load or a collection request fails.

Recent history is fetched afresh from Xbox and kept only in memory. Local launches no longer write a substitute Recently Played list. MRU pagination uses 50 results per request, follows continuation tokens, and rejects repeated tokens or more than 20 pages. Unavailable titles and duplicates are filtered without reordering the remaining entries.

The website also contains a feature-flagged TitleHub MRU implementation. This candidate uses the streaming service MRU: cross-device **cloud** history is the intended test. It does not claim to include every native console/PC launch or to match all personalized website sections. Live signed-in responses and public gallery responses still need device validation; only their client contracts and parsing/order handling have been verified here.

## Source verification

Inspected the public [Xbox Cloud Gaming website](https://www.xbox.com/en-US/play) and its loaded application bundles:

- [Streaming service client](https://assets.play.xbox.com/playxbox/static/js/8128.83844786.chunk.js): `enumerateMruTitles` calls `/v2/titles/mru?mr=...&ct=...`.
- [Client gallery definitions](https://assets.play.xbox.com/playxbox/static/js/client.7988fd9a.js): CloudRecentlyAdded `06323672-b8c8-43cc-b0de-32d5a9834749`; CloudMostPopular `6a589fa0-d493-472b-8e20-3813699d7056`.
- [Home page](https://assets.play.xbox.com/playxbox/static/js/home-page.c7943bbb.chunk.js): cloud gallery sections and the `EnableTitleHubMru` alternative.
- [Gallery client](https://assets.play.xbox.com/playxbox/static/js/2495.acf17ad9.chunk.js): `catalog.gamepass.com/sigls/v2` with market/language parameters and ordered product IDs. Requests to this public feed do not receive the streaming bearer token.
- [xbox-xcloud-player 0.2.11 source package](https://registry.npmjs.org/xbox-xcloud-player/-/xbox-xcloud-player-0.2.11.tgz), `src/Channel/Chat.ts`: microphone media tracks are added to WebRTC and SDP is renegotiated. Voice is not sent as arbitrary bytes on the chat data channel.
- [VitaSDK audio input API](https://docs.vitasdk.org/group__SceAudioInUser.html) and [SDL Vita capture driver](https://github.com/libsdl-org/SDL/blob/SDL2/src/audio/vita/SDL_vitaaudio.c): native audio input format and lifecycle.
- [Opus encoder API](https://opus-codec.org/docs/opus_api-1.5/group__opus__encoder.html): voice encoding and supported frame sizes.

## Validation and build gate

- `tests/feature-foundations`: 10 tests pass, including server order, response errors, old settings compatibility, mute invalidation and bounded audio queues.
- `tests/voice-codec`: native Opus roundtrip passes; production encoder packets decode to non-silent 20 ms audio at 48 kHz.
- `tests/rtc-transport`: 16 tests pass, including a real two-peer ICE/DTLS/SRTP microphone test, one audio media line, RTP timestamps/sequences, stale capture rejection and mute on disconnect/reconnect.
- The peer test injects encoded packets in place of hardware capture. It does not emulate Xbox chat services or Vita audio hardware.

After explicit user approval, the feature tree was published to `latency-root-cause` as `90c472f3f7793b1585e999ef528aa749103a1454`. [CI run 35729051338](https://github.com/jordanM333/green-vita/actions/runs/35729051338) passed all regression/feature test steps and the full native Vita release compilation, including UI integration and audio input/Opus linking. VPK conversion then failed because `vita-elf-create` needed 4,596 bytes of SCE metadata space between load segments.

The first follow-up used the newer [VitaSDK linker-script hook](https://github.com/vitasdk/buildscripts/blob/master/patches/binutils/0001-vita.patch), but the CI SDK did not apply it: [run 35730431634](https://github.com/jordanM333/green-vita/actions/runs/35730431634) still showed only 4,248 bytes between the executable and data load segments. The current wrapper uses a small GNU linker augmentation script, moving the location counter outside output sections before `.init_array` (the first writable section in the observed Vita ELF). This explicitly reserves at least 64 KiB, independently of that SDK hook. Unlike padding inside the executable segment, this leaves space at its end for module metadata. Native dependency compilation and streaming behavior are unchanged. The same adjustment is provided for Unix and Windows wrappers.

CI prints the resulting ELF segment layout and verifies the actual virtual gap, with regression tests reproducing the failed 4,248-byte layout. A host GNU linker smoke test using the augmentation script successfully preserved constructor execution and produced the intended gap. The Vita build remains the authoritative packaging check.

Successful packaging, actual Xbox negotiation, Vita hardware capture and audible voice remain separate build/device gates. RX37.4 installers do not contain this implementation.
