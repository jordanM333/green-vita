# Test 34: new recordings and history — issue OPEN

This update incorporates the supplied `IMG_0446.mp4`, `IMG_0450.mp4`, and pasted
history from 5.455 to 61.539 seconds. It supersedes the earlier request for a
simultaneous TV/Vita recording: **that evidence has now been supplied**.
No additional gameplay recording or installation is requested here.

The investigation has localized a persistent local decoder-residence increase
and repeated recovery freezes. It has **not** established the complete cause of
the reported 5–6-second response delay, or demonstrated a correction on hardware.
There is no candidate playback fix in this update.

## Source and session identity

- Runtime under investigation: `vita-streaming-v2`,
  `d7397bf4ccbcce08b7df9069b083be1f15306471` (RX Test 34).
- Investigation branch: `latency-root-cause`; this update builds on investigation
  commit `182ea8e57ebca2fabfc519c79cfe36bd71c334bb`.
- Both new clips contain the TV and Vita. The user identifies 0446 as without
  refresh and 0450 as after refresh. They are analyzed separately. Exact refresh
  time, elapsed gameplay duration before recording, and a unique correspondence
  between camera time and log time are not present. Do not assign either clip's
  individual events to guessed log rows.
- [Recording manifest](test34-recording-manifest.json) records hashes, original
  frame count, PTS and manually inspected event bounds. No footage is added to
  the public repository.
- [Selected history excerpts](test34-selected-history.txt) contain explicitly
  selected, verbatim lines from the supplied history, not a replacement full log.

At elapsed 5.455 s, RTP/SPS counters are empty while matched PTS is already 7,544,
resyncs 55, and decoder age 857/888 ms. These are inherited process-wide metrics
and a previous measurement window, not evidence that the new session started at
857 ms. The new stream becomes active at 7.458 s. Comparisons below use the first
steady output windows, approximately 8–11 s.

## What the recordings establish

Original frame cadence was inspected around an identifiable large explosion in
each clip. Frame numbers are zero-based, at 30 fps; timestamps are checked against
ffprobe's original presentation timestamps, not contact-sheet positions.

| Clip/event | TV onset bracket | Vita onset bracket | Vita minus TV, sampling bounds |
|---|---|---|---|
| 0446, large explosion at TV 9.4 s | Frames 281–282 | Frames 339–341 | 1,900–2,000 ms |
| 0450, first large explosion after refresh | Frames 60–61 | Frames 64–65 | 100–167 ms |

The first bracket is deliberately wider because the Vita's partial flash and
full flash span adjacent camera frames. Rolling shutter, exposure and annotation
threshold add uncertainty beyond these sampling bounds. These are **display
differentials, not absolute button-to-response or one-way network measurements**.
Button contact is not unambiguously visible for these effects.

The smaller early differential after refresh does not contradict the reported
failure: later in 0450 the TV turns into the other area while the Vita continues
to show the previous room. In 0446, the Vita holds the particle-filled room for
several seconds as the TV moves on. The recordings show variable delay and
freezes; an isolated prompt event after refresh is not a passing session.
Repeated similar flashes later in 0446 are not assigned an exact 5-second
measurement without unambiguous event correspondence.

The TV already displaying an event while the Vita has not displayed it rules out
delayed controller delivery as the **sole** explanation for that differential.
It does not establish prompt physical input delivery. TV processing delay also
remains part of the reference path. Neither clip measures Cloud performance or
provides a 30-minute acceptance run. Mixed TV/Vita audio cannot establish A/V sync.

## What the new history establishes

`Frame age: decoder` is matched output PTS → that AU's stored monotonic submission
time. It is different from the duration of the most recent hardware call. The
source records it after output-epoch validation. `receiveToGPU` follows a matched
picture to completed GPU presentation work, not physical panel scanout.

| Elapsed | Decoded per window | Decoder residence avg/max | Receive-to-GPU avg/max | Video arrival growth | AU queue age avg/max | Decode call avg/max |
|---|---:|---:|---:|---:|---:|---:|
| 8.460 s | 60 | 69/85 ms | 91/100 ms | 4 ms | 0/2 ms | 2/8 ms |
| 13.466 s | 0 | n/a | n/a | 27 ms | 0/0 ms | 0/0 ms |
| 19.475 s | 60 | 136/168 ms | 157/172 ms | 4 ms | 0/4 ms | 2/5 ms |
| 34.502 s | 59 | 170/195 ms | 192/201 ms | 11 ms | 0/2 ms | 3/6 ms |
| 48.516 s | 60 | 486/499 ms | 509/517 ms | 5 ms | 0/3 ms | 2/3 ms |
| 51.521 s | 60 | 486/497 ms | 509/517 ms | 2 ms | 0/2 ms | 2/4 ms |
| 55.526 s | 37 | 1,121/1,349 ms | 1,209/1,406 ms | 944 ms | 0/4 ms | 2/4 ms |
| 61.539 s | 60 | 620/639 ms | 638/647 ms | 2 ms | 0/3 ms | 3/4 ms |

These are once-per-second window statistics, not per-frame paired observations.
The 48–51 s plateau appears in all intervening supplied rows, approximately
486–487 ms decoder residence and 509–510 ms receive-to-GPU. It is not just a
one-window transient. The local decoder average is approximately **551 ms higher**
at the end than in the early steady windows. This already exceeds the proposed
100 ms stability target for this local component, but is not a measurement of
button-to-visible drift or of the whole reported 5–6 seconds.

The sequence visible at the available one-second resolution is:

1. Through 11.463 s, decoding is approximately 60 frames/s with 69 ms residence.
2. At 12.463 s, RTP loss/reassembly disruption and arrival growth are visible:
   assembly maximum 195 ms, video arrival growth 80 ms, audio growth 93 ms,
   and 592 ms waiting for a recovery IDR. At 13.466 s the recovery wait is
   1,595 ms with no new decoded/shown frames. The recorded completed wait reaches
   **2,405 ms**. This is a known source-code mechanism for holding the last image,
   not evidence of a 2.4-second decode call.
3. Recovery is not clean. There are additional waits, queue-full events, discarded
   generations, and bursts of IDRs. At 17.469 s a sampled ICE round trip reaches
   2,610 ms; input admission deferrals have increased substantially. Therefore
   input/transport trouble is not ruled out by `Input local: 1 ms`.
4. After arrival growth falls back to a few milliseconds, matched output remains
   older: approximately 136 ms, later 170 ms, and later 486 ms for sustained
   windows. The local software queue is empty at the snapshots and hardware calls
   remain short. This locates **retained delay between submission and returned
   decoder output**, not in a six-second SDL render queue.
5. At 55.526 s both video arrival growth and decoder residence are elevated.
   By 61.539 s arrival growth is again 2 ms, but decoder residence remains 620 ms.
   Thus arrival recovery alone does not restore the early local baseline.

The snapshots do not establish subsecond ordering within the initial disruption;
they do not justify claiming the decoder caused the transport trouble or vice
versa. Nor should the 2,405 ms wait, 2,610 ms RTT, 944 ms arrival growth and 1,121 ms
residence be added together: they concern different times, paths and populations.

Audio SDL backlog is 76 ms at 48.516 s and 85 ms at the end; Opus/PCM queues are
small. Underruns and trims increase. This neither explains a multi-second local
video delay nor proves audio is smooth/synchronized. The ~16,714 ms SR offset is
an uncalibrated clock difference. Raw V-A is not a lip-sync measurement.

The actual SPS is 1280×720, Main, one reference, reorder 0, DPB 1. The 3840×2160
server metadata is not evidence of 4K frames being decoded. Preserve 1280×720
initialization and 960×544 output.

## Source audit: supported mechanism versus remaining hypothesis

Two source paths matter together:

- `rtp.rs::record_damage` invalidates queued work on first damage; `Recovery`
  suppresses dependent frames until a complete IDR is admitted. This directly
  explains the logged IDR-wait freezes. Queue-full events can also enter this
  recovery path. Each recovery does **not** clear the firmware decoder.
- `worker.rs` calls the decoder only when receiving a new compressed AU.
  `decoder.rs` supplies `numOfElm = 1` and makes one `sceAvcdecDecode` call.
  Returned PTS is tracked separately from the current submission. Outputs from a
  prior generation can then be rejected without being counted as a successful
  decoded frame. Consequently `hwCall - decoded` is **not** hardware backlog;
  it includes old-epoch rejection, no-picture returns and errors.

At the 60-input/60-output plateaus there is no excess picture-output rate to
recover retained decoder delay. A missing independent output-service path is
therefore a concrete candidate mechanism for the persistent portion. However,
these logs do not reveal how many pictures were ready inside firmware, what
input it had consumed, or whether a zero-byte decode would return them. The
one-input/one-output source pattern alone is not proof of that firmware behavior.

Primary implementation comparison:

- [VitaSDK ABI](https://github.com/vitasdk/vita-headers/blob/5e1e7d38d766e4c1634a77f6e5249caab8c8f9cb/include/psp2/videodec.h)
  exposes input AU bytes, output capacity/count and picture PTS, without a documented
  readiness contract sufficient to settle this question.
- [Vita FFmpeg port](https://github.com/xfangfang/wiliwili/blob/88e5876bea9502d06f46a8656e3530684d3aaf7d/scripts/psv/ffmpeg/ffmpeg.patch)
  uses empty-input `sceAvcdecDecode` when its ES buffer lacks space, and a separate
  stop operation at end of stream. It also uses different internal decoder
  initialization. This establishes a relevant API usage, not that adding a drain
  loop to this build is safe/effective on the user's Vita.
- [Moonlight's Vita implementation](https://github.com/xyzz/vita-moonlight/blob/984603bd6f93f752593048fe494b5ffca14514e1/src/video/vita.c)
  also uses one output slot per decode call. Its different stream does not prove
  this pattern is sufficient or insufficient for Xbox.

No simulated decoder that simply implements the hypothesized queue would prove
that the real firmware behaves that way. No drain/reset experiment is added to
the runtime branch on that basis. The distinction remains: a local failure
interval is measured; the complete corrective mechanism is not yet demonstrated.

## Experiment disposition

The earlier [34-build audit](ROOT-CAUSE-REVIEW.md#change-history-and-experiment-disposition)
continues to apply. The new evidence changes these assessments:

| Change | New assessment |
|---|---|
| Test 34 SCTP admission correction | Its off-device starvation regression remains valid. It did not resolve the current live symptom; do not promote it to the remaining root cause. |
| Test 34 adaptive REMB | Requested target reaches 500 kbps while stalls and local residence growth persist. No measured gameplay resolution. It cannot be justified as a permanent fix from the synthetic bottleneck model. Keep it frozen as the exact observed baseline during trace analysis; an eventual minimal candidate must justify retaining it or isolate its removal. |
| Three-AU/50 ms policy and generation recovery | These bounds do not bound firmware residence. Queue-full recovery/old-epoch discards need per-event analysis before changing limits or reordering. |
| Removed Test 31 reset/reconnect policy | Keep removed. Restarting accumulation is not a fix. |
| PTS identity, texture ownership, decoder capacity, authentication, mode support, controls and debug toggle | Preserve. Current investigation tooling does not alter their behavior. |

## Work implemented and verified in this update

The offline analyzer now extracts local metrics from history while marking
inherited startup windows, retaining no-output windows as missing measurements,
and keeping process counters separate from per-session observations. It audits
raw decoder-return PTS against earlier submissions, including output that was
subsequently rejected by generation checks. It handles RTP wrap, unknown PTS,
duplicate returned PTS, ambiguous repeated input timestamps and truncated rings.
It explicitly refuses to call unmatched metadata a hardware queue count.

Fourteen analyzer tests pass, including six new cases for those distinctions.
This is **analysis-tool validation only**. It is not a playback regression test,
Vita compilation, an on-device result, or evidence that the issue is fixed.

Reproduce the history extraction:

```sh
python3 tools/analyze_latency.py --history investigation/test34-selected-history.txt
python3 -m unittest discover -s tests/latency-analysis -v
```

## Missing existing evidence — no new test/install

Please provide these files from the Test 34 sessions already recorded:

```text
ux0:data/green-vita-540-test/pipeline-trace.csv
ux0:data/green-vita-540-test/pipeline-previous-trace.csv
```

The source saves them on stream shutdown and retains the preceding session on
refresh. The previously available CSV predates matched-output diagnostics and
cannot substitute for these Test 34 files. This is a request for already-created
evidence, not to repeat gameplay. If those exact session files have been
overwritten, state that; do not silently substitute another session.

Specific question: **During the retained delayed interval, does each fresh AU
submission return a progressively older PTS, and how do no-picture returns and
old-generation rejections change that sequence?** The raw rows distinguish these
cases; the history averages cannot. The retained 4,096-event ring may cover only
several seconds and cannot reconstruct a missing onset or establish internal
firmware readiness. If the raw sequence still cannot establish the corrective
mechanism, that limitation must be reported rather than papered over with a VPK.

## Acceptance remains unchanged

No new installable candidate is offered. Test 34 remains the identified installed
baseline, not a known-good build. Retained older VPKs are rollback artifacts, not
proof of acceptable latency. Runtime, transport, buffering and codec settings
have not changed in this update.

A future candidate must be labeled **candidate fix—on-device verification
pending** until Cloud and Home each pass two independent uninterrupted 30-minute
runs, repeated early/late absolute button-to-visible measurements with at most
100 ms added latency, no concealed recovery cycle, and no material playback,
audio-sync or controller regressions. Idle/resume and network-disruption recovery
remain separate conditions. A stable sluggish baseline fails. No platform limit
has been established.
