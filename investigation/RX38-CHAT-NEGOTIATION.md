# RX38 follow-up: Xbox microphone negotiation and startup recovery

Follow-up: [RX38.10 bottom controls, stream volume and recovery](RX38-CONTROLS-AUDIO-RECOVERY.md). The user confirmed RX38.9 voice works without echo complaints.

**RX Test 38.9 is built and verified.** Install `GreenVita-RX-Test-38.9.vpk`
over the RX Test app (`GRNVTEST1`). [CI run](https://github.com/jordanM333/green-vita/actions/runs/35812410990)
passed all 132 test executions, native Vita compilation, VPK conversion and ELF
metadata headroom checks. Source: `05720eee93134489db44f9809066ebb9822b02b5`.
VPK SHA-256: `65edf071c7a8c145b9828c7a3c5529198d7d71dd8a42668c44632f91ae1549ea`.
Artifact checksum, embedded source/build identity, ZIP integrity and title ID
were verified. Only `eboot.bin` differs from RX38.7. See
`CANDIDATE-38.9-BUILD.json`. Device acceptance remains pending.

RX38.8 stopped at a new test assertion that incorrectly inspected the whole SDP,
including the bidirectional application channel. The assertion now checks the
audio media section explicitly; the passing RX38.9 run includes the full real
peer negotiation and failure-recovery checks. No RX38.8 installer was produced.

## Diagnosis and changes

RX38.7 added a sending track to the initial streaming offer. Its packet count
reported local sender admission, not Xbox microphone recognition. Inspection of
the reference clients shows an explicit chat SDP exchange after adding the track:

- [xbox-xcloud-player chat](https://github.com/unknownskl/xbox-xcloud-player/blob/main/src/channel/chat.ts)
- [xbox-xcloud-player stream](https://github.com/unknownskl/xbox-xcloud-player/blob/main/src/lib/stream.ts)
- [Greenlight session API](https://github.com/unknownskl/greenlight/blob/main/packages/desktop/main/helpers/xcloudapi.ts)

The initial game offer is now receive-only. First unmute adds the Opus sender to
the existing audio media line and exchanges SDP at the current Home/Cloud session
endpoint with `configuration.isMediaStreamsChatRenegotiation=true`. Capture waits
for an accepted answer. The HTTP exchange runs alongside the RTC pump and has a
15-second timeout. A failure rolls back the pending offer and disables optional
voice for that session, without deliberately stopping game media. A late answer
never reverses a user mute. Subsequent mute/unmute reuses accepted chat.

A semi-transparent button at the top left toggles the same microphone state as
the quick menu. It distinguishes MIC OFF, CONNECTING, MIC ON and MIC N/A. Touches
starting on it are consumed through release, including a drag off the button;
they are not forwarded as game pointer or front-touch auxiliary buttons. Extra
fingers and SDL's synthetic mouse copy cannot toggle it twice. Backgrounding and
new sessions remain muted.

If no picture has been produced after 30 seconds, a visible startup prompt gives
the recovery path: Quick menu → Refresh stream for Home; Quick menu → Exit game
and launch again for Cloud. Waiting continues and a late picture clears the
prompt. This does not automatically restart a remote game or treat ordinary
in-game pauses and later packet loss as failed startup. Decoder, bitrate,
reordering, keyframe and rendering policies are unchanged.

## Device check

1. Start Home. The button should be MIC OFF once connected. Join voice chat via
   the game's or Xbox's controls, then tap the button.
2. Expect CONNECTING, then MIC ON after the chat answer. Check Xbox's microphone
   indicator and have a friend confirm audible voice. The input meter and local
   RTP counter alone do not prove that a friend can hear you.
3. Tap Off while speaking; verify silence, then unmute and verify no old speech
   replays. Also mute while CONNECTING and confirm the eventual answer leaves it
   Off. A connection error is shown in the quick menu; report its exact text.
4. Test a drag off the mic button, a second simultaneous finger, front-touch
   auxiliary controls, background/return, and a fresh session. Check for unwanted
   game button presses and that background/return never unmutes automatically.
5. Repeat in Cloud. Compare game audio, video and controls before/during/after
   mic activation. Use headphones for the first voice check; no acoustic echo
   cancellation is implemented.
6. If startup remains blank for 30 seconds, confirm the recovery message appears
   even with diagnostics hidden. Do not deliberately disrupt a live game just
   to force this condition.

## Automated coverage

The two-peer test exercises actual ICE/DTLS/SRTP with initial receive-only SDP,
later chat offer/answer, one audio media line, game audio and controller traffic
while negotiation is pending and after it finishes, mute during negotiation,
RTP timing, stale capture rejection, and rollback after a failed chat exchange.
The test peer is not an Xbox service. Hardware capture remains a device gate.

Unit coverage includes chat request configuration, touch gesture ownership,
capture gating, and per-session startup timing with late-picture recovery. The
existing video/transport/codec/catalog and Vita packaging gates also run. This
build does not claim to resolve delays caused by the guest network or extender.
