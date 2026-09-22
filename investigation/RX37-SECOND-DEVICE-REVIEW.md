# RX37.4 second capture: delivery delay starts early

## User clarification

Only the game was paused; GreenVita's menus were not open. This rules out the
GreenVita pause-overlay branch as the trigger for that reported episode. It
does not independently establish whether the device screen dimmed or slept,
but a suspend/resume change is not justified by the clarification alone.

Jordan also reports delays during Cloud play. The second attachment itself
does not record Home/Cloud mode, so keep that report separate from a confirmed
mode label for this particular capture. Both modes share the Vita receiver.
The Xbox's Ethernet connection to an extender is relevant to Home; Cloud also
depends on the Vita's access point and Internet path.

## Evidence identity

- Build: RX Test 37.4, runtime 70ccd93aee1c99bb8550344c264b0b6413ece1d9.
- `pipeline-history(1).txt`: SHA256
  `21c37ea577e52905e6056f22587d159c8ea58f2e5fb226f50ba8321db0a7bd88`.
- `pipeline-incidents(1).csv`: SHA256
  `38ed595547c2de514a449ed9b292c2e06d6ac77a08f0a52f5f215618f83d2280`.
- History: 752 snapshots, 2.875-764.170 seconds; 748 after video startup.
- Incident ring: 1,024 events, 311.563-763.127 seconds. Earlier freezes are
  outside this retained ring.
- Actual H.264 SPS: 1280x720, Main level32, refs1, reorder0, dpb1.

## What the capture establishes

| Evidence | Observation |
|---|---|
| Delay starts shortly after video begins | At 6.881s video arrival growth 354ms/audio 321ms; at 8.882s 2033/2244ms; at 12.885s 5406/5235ms |
| The receive loop remains active during that slowdown | At 12.885s 425 receive passes, zero receive-budget hits, at most 1 datagram/pass, RTC pump max 2ms, compressed-frame queue 0 |
| The same failure grows larger later | At 264.235s video arrival growth 8892ms and latest audio growth 7760ms, queue 0, 422 receive passes, budget 0, pump max 2ms |
| Lost reference recovery prolongs the freeze | Completed recovery maximum 31422ms; final 106 resyncs, 33 queue-full drops, 144 sequence-damaged AUs, 54 FU-damaged AUs, 4178 IDR-wait drops |
| The decoder is usually able to keep up when media arrives | Median window decoded 59/s, shown 57/s, AU queue depth 0, average decode call 3ms |
| Measured local output age is usually much smaller | Across 643 windows with measured output: median decoder residence 58ms, median receive-to-GPU 67ms |
| Burst handling remains a secondary problem | Queue reaches 32 and drops whole reference chains during catch-up; this cannot explain the earlier joint arrival slowdown while the queue is empty |
| This run eventually recovers | Final window decoded 61/s, shown 50/s, queue 0, video/audio arrival growth 27/26ms; decoder errors 0, unmatched returned PTS 0 |

Relative arrival growth measures added delay against this capture's fastest
observed RTP arrival. It is not calibrated button-to-screen latency. The audio
and video maxima need not refer to the same instant. A field that stops
receiving new samples can remain unchanged; do not treat it as a live probe.
Window averages and their medians are not per-frame latency percentiles.

The incident tail's largest fresh-picture gap is 4.809s ending 320.926s. Its
counts include 52 recovery starts / 52 ends and 2 frame-limit events; neither the
gap maximum nor those counts cover the first 311 seconds. In particular, the
31.422s recovery maximum comes from history, not that incident tail.

The current input transport line is `sampled current / process maximum` bytes.
The 4908B maximum remaining unchanged is not evidence of a permanent 4908B
backlog. Final sampled outstanding payload is 172B. Rejected input admission
attempts also count retries and are not unique dropped button presses.

## Diagnosis and remaining boundary

The evidence supports a delivery slowdown before AU admission, followed by
packet loss and bursts that amplify the disruption through IDR recovery. It
does not isolate Xbox/server sending, the access point/path, Vita networking,
or readiness handling. The game-pause correlation does not explain the
early-start slowdown in this second capture.

Source review found no RTC pause command or game-pause detection. SDL idle
timer prevention is separate from Wi-Fi power saving. Neither automatically
adding an idle tick nor flushing the decoder follows from these observations.
Likewise, passing host transport tests does not validate Vita socket readiness
or access-point behavior on the device.

## One controlled next comparison

Sony's [Vita user guide](https://manuals.playstation.net/document/en/psvita/settings/wifi_savemode.html)
documents **Settings > Power Save Settings > Use Wi-Fi in Power Save Mode**
and says disabling it can improve an unstable access-point connection.
The user's current setting is not known; this is a testable hypothesis, not a
diagnosed cause or guaranteed fix.

1. Keep RX37.4 and the current network unchanged. If the Wi-Fi power-saving
   checkbox is enabled, turn it off and start a fresh Cloud session.
2. Play for five minutes, including normal movement; note whether the long
   delay returns. Preserve the setting's previous state and label the result
   explicitly Cloud / Wi-Fi power save off.
3. If it was already disabled, the alternative comparison is one Cloud run on
   a phone's cellular hotspot (phone Wi-Fi off so the home network is bypassed).
   That comparison changes the path, not the application. It cannot by itself
   identify which component of the old path was responsible.

No new runtime or installer is represented as a remedy from these logs alone.
If neither comparison changes the outcome, the next targeted instrumentation
must distinguish actual OS receive attempts/WouldBlock from cached reactor
readiness, record gaps between RTC pump entries, and record mode and selected
route. Another decoder queue threshold is not supported by this evidence.
