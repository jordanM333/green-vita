# Bottom controls, stream volume and recovery follow-up

Verified candidate: **RX Test 38.10**, source
`0286e874ab1ba5081e1d9b66325f5dbcaaa18bb7`.
All 139 test executions and native Vita packaging passed. VPK SHA-256:
`d284eda2d23d6db9e309e0f51976316db54349740a68dc3a9768de819c4d9aa6`.
The embedded source/build, ZIP integrity, artifact digest and title ID were
verified. Only `eboot.bin` differs from RX38.9. Full build identity is recorded
in [CANDIDATE-38.10-BUILD.json](CANDIDATE-38.10-BUILD.json).
Device acceptance of these new controls and recovery changes remains pending.

Baseline: RX38.9, source 05720eee93134489db44f9809066ebb9822b02b5.
The user reports working voice transmission with no echo complaints and excellent
initial playback. Preserve that chat negotiation and microphone capture path.

## Changes

- Microphone at bottom left, Xbox/Guide at bottom center. Both use the same
  112x38-point rounded surface, black alpha 60/255 and foreground alpha 180/255.
  Mic state still distinguishes off, connecting, active and unavailable.
- Both controls share their painted geometry with front-touch/mouse routing.
  Overlay gestures remain owned through release outside the button, secondary
  fingers cannot trigger clicks, and game-origin gestures remain game gestures.
  The Xbox action reuses the existing Guide press/release pulse.
- Settings > Audio > Stream volume: persisted 0-100%, default 100% for existing
  settings. Touch slider or D-pad left/right in 5% steps. Stereo PCM gain uses a
  10ms ramp, bounded at unity, without changing microphone gain or negotiation.
  Already queued PCM plays at its previous gain until consumed (existing bounded
  playback queue); this is not the microphone mute gate.
- This transport has one incoming mixed game/chat audio feed. The volume control
  intentionally identifies itself as stream volume. Separate game and incoming
  voice adjustment must happen in the game's audio options or Xbox audio controls.
  Browser Xbox party chat uses a separate party session; adding such a session is
  a separate feature, not achievable by two gain sliders over this one stream.
- Receive-budget decisions use the minimum delay within each 200ms window to
  avoid treating a single late boundary packet as sustained congestion. Persistent
  congestion still reduces the target, with the same 500kbps floor and 2Mbps cap.
  Stable 41-80ms paths may cautiously probe +100kbps after 10s; low-delay paths
  retain the 5s recovery interval. Rising delay resets the recovery interval.
  Idle receive gaps do not count as evidence of spare bandwidth. The clock
  baseline is preserved; no accumulated delay is hidden by rebasing it.
- Home shows manual refresh guidance after three seconds of fresh measurements
  at least 500ms behind. A recovered or stale measurement clears the hint.
  It does not restart a session or disrupt voice automatically.

## Device evidence and limits

The supplied RX38.9 Home history covers roughly 1.2-365.7 seconds. The initial
zero-RTP snapshots contain inherited process counters, so they are excluded from
the current-session baseline. Actual H264 SPS is 1280x720; the server's 3840x2160
metadata is not evidence of decoding a 4K stream.

Through roughly 305s, playback is near 60 pictures/s, receive-to-GPU averages
around 63-68ms, and relative video arrival growth is usually single digits.
At 309.6s, relative video arrival growth is 96ms; by 313.6s it reaches 1080ms,
while receive-to-GPU averages 153ms. Later measurements repeatedly exceed 1s
of arrival growth. Local draw/queue work remains much shorter. The microphone
is off throughout this degraded stretch; earlier voice activity is recorded.

The requested REMB target decreases from 2000 to 500kbps over 310-314s, but
measured video payload often remains above 2Mbps. Feedback is a request, not a
confirmed enforced sender limit. Every receive pass hits its work budget during
much of the degraded period; this log alone cannot separate sender delay, network
queueing and socket backlog. Do not attribute all delay to the router or GPU.

Retained incidents span 161.862-361.034s: 38 completed recovery waits, median
191.5ms, maximum 1596ms, and new-picture gaps up to 2049.833ms. At 360.7s an
arrival burst raises the decoder queue to 24; it reaches the 32-frame safety
limit once, causing another recovery. Arbitrary P-frame dropping would break
the reference chain and is deliberately not introduced.

At 361.7s arrival growth falls to 59ms, then settles at 66-71ms through the end
of the log. The old <40ms recovery requirement would prevent bitrate recovery
if this level persists. The log ends before the new 10s recovery interval would
elapse, so recovery on hardware is still a device test. The retained full trace
is only the final 3.07 seconds, not the earlier disturbance.

These changes address bitrate recovery and provide a visible recovery route;
they do not claim to fix all causes of the one-second upstream delay.

## Verification and device acceptance

Host regressions cover both button hit regions at Vita scale, multi-touch
ownership, gain bounds/stereo/ramping, backward-compatible settings, persistent
lag detection, the settled 70ms recovery case, isolated boundary jitter and a
virtual bandwidth collapse. Existing real-peer voice negotiation tests remain
required, followed by the native Vita build and VPK integrity checks.

On device, verify a tap on Xbox opens/closes the Guide once, mic taps only toggle
voice, and both overlays leave other front-touch controls usable. Check stream
volume at 100/50/0%, restart persistence and unchanged outgoing voice volume.
Repeat Home after the network disturbance; watch whether relative delay actually
falls and whether the bitrate climbs after a stable path, rather than judging
recovery from a low local decoder queue alone. Cloud requires its own session.
