# RX37.4 third capture: confirmed Home, substantially improved delivery

## Identity and comparison conditions

Jordan explicitly identifies this capture as **Home streaming**. Runtime is
RX Test 37.4, revision `70ccd93aee1c99bb8550344c264b0b6413ece1d9`.
The preceding device photo shows Wi-Fi power saving enabled. Jordan now confirms
this Home capture was made with that checkbox **unchecked** and reports better
overall gameplay. The comparison supports leaving it disabled, but the runs
are not a controlled isolation of every network condition. Cloud testing is pending.

- `pipeline-history 2.txt`, SHA256
  `5980e72a8b3c8b9a76d52ca9a4a6db85ffc03106b795fed42db3a443a46d50f3`.
- `pipeline-incidents 2.csv`, SHA256
  `790b5670a2c544e32ff5eb0cf56aa9dcb127826f80e0d9509b0b462645a78261`.
- History: 864 snapshots, 1.137-870.693s; 862 after video startup.
- Incident ring: 1,024 events, 120.278109-865.485152s. It does not cover the
  first two minutes and is not a complete session event count.
- SPS remains 1280x720, Main level32, refs1, reorder0, dpb1.

## Findings

The earlier multi-second arrival slowdown affecting both audio and video is
absent from these sampled windows. This is a substantial improvement on the
same runtime, but residual video stalls still fail a smooth-play acceptance.

| Measurement | Earlier likely-Home capture | This confirmed Home capture |
|---|---:|---:|
| Last history time | 931.567s | 870.693s |
| Maximum video relative arrival growth | 5930ms | 1621ms |
| Maximum audio relative arrival growth | 5985ms | 264ms |
| Windows with both arrival-growth values at least 1s | Present in 11 stretches | None |
| Video reorder missing/released packets | 5546 | 132 |
| Audio lost packets | 2141 | 5 |
| AU queue-full drops | 25 | 4 |
| Completed reference-chain recoveries | 150, plus one active at end | 73, none active at end |
| Longest fresh-picture gap in retained incident ring | 9500.951ms | 1447.138ms |

These are descriptive comparisons across different runs and retained spans,
not normalized loss rates or proof of a network-setting effect. Relative
arrival growth is against each stream's fastest observed arrival, not calibrated
one-way network or button-to-screen latency. Maxima can occur at different
instants, and a latest-value probe is not a continuous measurement.

Across this run's 862 video windows, median decoded rate is 60/s and shown rate
59/s. Median window-average decoder residence is 56ms and receive-to-GPU age
65ms; their window-average p95 values are 85ms and 110ms. Median compressed-AU
queue depth and average queue age are zero; average queue-age p95 is 5ms.
These window statistics are not per-frame latency percentiles. There are 84
windows with shown rate below 50/s, so the median must not hide the stalls.

Quality varies within the run: the first two minutes already contain 31
completed recoveries. From 120-800s the median shown rate is 59/s and video
arrival-growth p95 is 187ms. The last 71 sampled seconds contain seven more
recoveries and video arrival-growth p95 rises to 1399ms, while audio remains
below 125ms in that segment.

## Remaining failures

The two largest retained fresh-picture gaps have specific packet-gap triggers:

1. At 821.908327s the reorder layer releases a one-packet gap. At 821.914462s
   a sequence-damaged AU starts recovery. Five keyframe requests follow.
   Recovery ends at 823.269452s after 1354ms; the fresh-picture gap ending
   823.366261s is 1447.138ms.
2. At 841.754141s another one-packet gap is released. Recovery begins at
   841.793379s, ends at 843.098093s after 1304ms, and produces a 1420.537ms
   fresh-picture gap ending 843.181496s.

This establishes that packet-gap handling and the wait for a usable keyframe
prolong those freezes. It does not determine where the packet went missing,
whether a later packet could have repaired it, whether the sender received
each request, or whether a retransmission would arrive within a useful budget.
Increasing the reorder deadline or requesting more keyframes is not yet a
demonstrated remedy.

All four frame-limit events are retained: 120.636330s, 827.233419s,
847.753586s, and 865.125873s, each at 32 queued AUs. The latter three occur
after the late-session delivery disruption. Queue overflows remain a separate
contributor; they do not explain the two packet-triggered freezes above.

Unlike the earlier starvation stretches, some windows here reach the receive
time budget on every pass. At 822.605s, 243/243 passes hit the budget, with at
most 14 datagrams/pass and pump max 6ms. At 842.654s it is 253/253, at most
14 datagrams/pass, pump max 10ms. The cap is checked after packet processing,
so elapsed times can exceed 2ms. A budget hit alone does not establish unread
OS backlog or prove that increasing the cap would improve total latency.

## Counter scope and final state

This stream starts in an already-running process. `resync` begins at 66 and
ends at 139: the session delta is **73**, consistent with the session recovery
counter, not 139 new recoveries. Decoded PTS matches likewise increase from
65483 to 115998, a delta of 50515. Decoder errors, unmatched PTS, and resets
remain zero. The inherited initial queue maximum of 25 is not a startup queue.

Final AU drops total 1102: 75 sequence, 11 FU, 4 queue-full, 1012 IDR-wait.
The final window has queue depth 0, decoded 59/s, shown 54/s, relative video/
audio arrival growth 1/8ms, and no active recovery. Input admission averages
0-2ms across the video windows, with no deferred admissions or send errors.

## Next decision

The checkbox state and perceived improvement are confirmed. Preserve RX37.4
as the Home comparison baseline; no new runtime
change is justified as a proven cure by this upload. The remaining engineering
targets are bounded packet-gap repair/recovery and burst service. Instrument
actual receive attempts versus cached readiness, inter-pump gaps, and selected
route before assigning a cause to the remaining delivery delay. This Home
result does not establish Cloud performance.
