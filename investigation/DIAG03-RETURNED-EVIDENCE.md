# DIAG03-8 returned device evidence — issue remains open

## Identity and preservation

The four returned files identify Home, DIAG03-8, source `af53e01bb3a16cba3add48023396fb8fa0849169`, epoch 1. They were analyzed without modification. The branch was fetched before continuing: `architecture-review-20260926` still pointed to `471a1fc0707edbce920dafc44681db6d9d1fc14b`; no newer branch work was displaced. This is a continuation of the diagnostic investigation, not a restarted architecture audit.

| File | SHA-256 |
|---|---|
| manifest.json | af7817b1d98dd83ce41023eec4cd6c17cdaade99b48bfcf5175dcc6fc95517e8 |
| events.csv | 7a630fd5f37d656fc5d38940c018025ade2db236003bb0f3459897e8308dfa1b |
| history.txt | 885d6a66fed9e5f9bd5c46ce3ceea63f671dc147d87f197225071374e1dd1ae8 |
| README.txt | 7a377df5907cdd68d1ffabefd0a22a8ded6acd7ea65f9f2fbb78eed973aeaf9e |

First video is at 2.710601 s, trigger at 115.264518 s, freeze at 125.264687 s. The capture contains 88,185 events, no capacity truncation, and 2,268 recorder lock skips. Those skips are counted over the recorder lifetime, not just retained windows; dividing them by retained events would not give a valid loss percentage. Unknown samples remain unknown. Of 6,713 authenticated nonempty video packet observations, 6,704 have a unique original UDP record; nine lack it. Audio has 1,242 observations and 1,237 matches. The retained initial window and approximately 103–125 s onset/aftermath window must not be interpreted as continuous packet coverage across the intervening gap.

The returned files establish that the device ran this build and saved/exported an incident with its onset intact. They do not validate every possible exit path or quantify recorder CPU overhead relative to an uninstrumented device.

## First growing boundary

All times below are monotonic elapsed device times, not synchronized wall-clock timestamps. `D` is application UDP dequeue, `L` is authenticated RTC delivery, `E` is the before-syscall timestamp of an actual empty-socket probe. Neither D nor L is network arrival. This capture records a maximum 1.620 s added video displacement before dequeue; the prior captures' roughly 2 s must not be substituted for this capture's observed maximum.

| Interval | Video dequeue/media growth, median | Video media / dequeue wall progression | Video socket residence upper bound, max | Same-packet RTC residence, max | Audio dequeue/media growth, median |
|---|---:|---:|---:|---:|---:|
| 112–113 s | 1.184 ms | 1.000 | 8.203 ms | 2.801 ms | 112.292 ms |
| 115–116 s | 639.106 ms | 0.492 | 13.587 ms | 2.977 ms | 112.543 ms |
| 116–117 s | 1,140.796 ms | 0.636 | 18.513 ms | 2.980 ms | 112.872 ms |
| 119–120 s | 1,444.656 ms | 0.970 | 18.924 ms | 2.898 ms | 113.654 ms |
| 124–125 s | 1,420.024 ms | 0.694 | 1,712.190 ms **upper bound only** | 2.879 ms | 198.048 ms |

These are first-observed advancing video timestamps per one-second window, not all fragments. Audio's earlier approximately 112 ms baseline displacement is already present in the pre-onset window; it must not be described as zero latency. Its stable progression during the video onset is the relevant comparison.

An independently checkable packet pair proves that most of the video displacement exists before the observed UDP socket:

* Baseline: epoch 1, SSRC 2912931032, sequence 28870, RTP 3540336571, D = 112.530519 s.
* Delayed: same epoch/SSRC, sequence 31191, RTP 3540965671, D = 121.118300 s, L = 121.120969 s, E = 121.115211 s.
* Wall progression is 8.587781 s; RTP progression at 90 kHz is 6.990000 s. Added displacement is 1.597781 s.
* Socket residence is at most D−E plus quantization = 3.090 ms. Allowing another microsecond for displacement rounding gives **at least 1.594690 s additional displacement before this UDP socket**. Baseline socket residence cannot be negative.
* Actual measured D→L is 2.669 ms. This is not a queue-depth inference.

There are 481 advancing video observations with a positive pre-socket growth lower bound of at least 500 ms. The first such witness occurs at D = 115.261689 s: added displacement 513.170 ms, socket upper bound 8.214 ms, conservative pre-socket lower bound 504.955 ms. Missing observations are not assumed to share those bounds. No observed video timestamp discontinuity or wrap explains this incident. Video sender-report RTP/NTP pairs maintain 90 kHz; inspection of all short report intervals also confirms the mapping, beyond the analyzer's longer-pair rate check.

Between 114.4 and 116.4 s, observed video media progresses at 0.515 media seconds per dequeue wall second. The deficit is approximately 485 ms/s. The application handles about 224 nonempty video packets/s in that interval; packet production/network arrival rate and pre-socket backlog size remain unmeasured. Completed AU median size rises from 2,764 bytes (104–112.5 s) to 8,102 bytes (114.4–116.4 s). Completed compressed work is about 1.96 Mbit/s of wall time but 3.87 Mbit/s of video media time. This supports a compressed-work/service mismatch **before dequeue**, but cannot distinguish sender pacing/encoding from selective network or driver delay.

## Downstream residence and the later overload

| Measured interval | Healthy 104–112.5 s, median / max | Rapid rise 114.4–116.4 s, median / max |
|---|---:|---:|
| First dequeue → AU completion | 2.715 / 48.991 ms | 40.719 / 53.123 ms |
| Actual H.264 assembly | 0.150 / 46.824 ms | 38.413 / 51.984 ms |
| Completed AU queue wait | 0.076 / 14.018 ms | 0.079 / 6.237 ms |
| First dequeue → decoder submission | 2.895 / 49.247 ms | 40.915 / 53.384 ms |
| Decoder submission → matched picture output | 54.928 / 108.316 ms | 101.088 / 145.400 ms |
| First dequeue → GPU completion | 68.759 / 122.045 ms | 151.062 / 196.174 ms |
| Audio dequeue → SDL submit | 41.159 / 65.872 ms | 33.072 / 66.445 ms |

GPU completion is not scanout and SDL submission is not audible output. Frame cohorts are only associated when epoch/track/SSRC/RTP identity is unique. Audio legacy downstream events lack a usable RTP identity, so their residence samples are interval-level observations, not falsely joined audio packets.

During the rapid rise, receive-pass maximum is 3.319 ms, between-pass maximum 4.105 ms, and 822 passes execute in two seconds. Recent actual empty checks and direct residence measurements exclude seconds of socket or RTC accumulation for the correlated packets. These do not prove zero CPU contention; between-pass time includes idle waiting.

After the local catch-up request at 122.405652 s, traffic rises sharply. The last retained empty observation is near 123.283751 s; the 124.354 s history snapshot reports zero empty observations in its latest window and all 349 receive passes hitting the work budget. Incoming RTP is 8.933 Mbit/s, with 748 ms spent in RTP processing during that window. Audio displacement grows too. This is a later receive-load condition. A 1.7–1.9 s socket upper bound then loses discriminatory power; it does **not** establish that packets actually resided there for that long. It cannot retroactively explain the onset, which occurred while the socket repeatedly emptied.

## Feedback and negotiation audit

The diagnostic preserved the 49e6 timestamp behavior and all bitrate, resolution, receive budgets, priorities, IDR and recovery thresholds. The returned capture quantifies the changed D→L origin as a few milliseconds at onset, not the missing second.

* TWCC continues at approximately ten successful local UDP sends/s; receiver reports at two/s and REMB attempts at two/s. Successful encrypted UDP sends do not establish the contents observed by Xbox or its reaction.
* The first ceiling reduction is at 115.537620 s, after 632 ms of growth: 2,000,000 → 1,400,000 bit/s. Subsequent reductions occur at 116.635716, 117.731200 and 118.819546 s. Thus the reductions cannot initiate this incident. Delivered traffic subsequently exceeds the requested ceiling; this does not, by itself, identify whether old work is draining or the sender ignores a constraint.
* The first captured catch-up/PLI is at 122.405652/122.405670 s. The first captured reference recovery is at 123.996305 s. They cannot cause the earlier 114–116 s growth. Recovery may contribute to the later burst; encrypted payload/IDR response and sender decisions are not fully captured, so causation remains a hypothesis.
* Captured frame feedback has no failed admission during the healthy or rapid-rise intervals. Its frequency falls with presentation (492 retained reports in 8.5 healthy seconds; 60 in the two-second rise), with no multi-second gap. SCTP FWD-TSN and T3 counters remain zero in these history windows. Per-frame receipt/application at Xbox is not measured.
* Source inspection confirms the metadata field order and common monotonic millisecond origin against [xbox-xcloud-player packet serialization](https://github.com/unknownskl/xbox-xcloud-player/blob/a6511a645736f4988f77323284c6f0f36bf93aef/src/channel/input/packet.ts) and [frame timing](https://github.com/unknownskl/xbox-xcloud-player/blob/a6511a645736f4988f77323284c6f0f36bf93aef/src/render/video.ts), both pinned to commit `a6511a645736f4988f77323284c6f0f36bf93aef`. GreenVita reports its measured decode/render stages rather than that reference's browser estimates. This comparison validates neither proprietary sender interpretation nor sender queue limits.
* The production TWCC interceptor uses the original packet Instant, 250 microsecond delta units and a 64 ms reference unit. Existing real ICE/DTLS/SRTP/SRTCP tests were rerun and verify decoded TWCC/REMB at a peer, including negotiated extension binding, wrap and decline. The capture does not retain the actual transport-wide extension values or decrypted outgoing feedback; a synthetic-peer test cannot certify those device bytes or emulate Xbox's congestion controller.
* The first-packet SSRC binding, REMB SSRC/bit units, unchanged negotiated ceiling handling, and Home/Cloud shared peer path were inspected. No new demonstrated protocol defect explains this onset. The 30 FPS / max-mbps requests coexist with approximately 60 received media frames/s before onset; requested capability is not evidence of compliance. Changing those requests now would be another uncontrolled experiment.
* Video sender-report receive-time progression versus reported NTP stays within about 11 ms of its pre-onset baseline during the rapid rise, while video media displacement grows by a second. Together with audio, this argues against a uniform shared FIFO delay affecting all datagrams identically. It does not exclude media-selective sender, network, QoS or driver behavior, or an inconsistency between media timestamps and actual capture time.

## Issue register and implementation decision

| Classification | Evidence / mechanism / affected boundary | Consequence / confidence | Action and verification |
|---|---|---|---|
| UNPROVEN HYPOTHESIS for the mechanism; boundary measurement confirmed | Correlated packet bounds demonstrate at least 1.594690 s growth before the observed UDP socket | Old media reaches every otherwise fast downstream stage; high confidence under the verified nominal RTP clock model | Preserve explicit packet witnesses and time-window analysis; no downstream queue-size change can remove this missing upstream interval |
| ARCHITECTURAL WEAKNESS | Later traffic surge, continuously occupied socket, receive budget saturation and increased audio displacement | Later client load can add delay; actual socket residence unknown | Keep separate from onset; no arbitrary receive-budget/priority tuning |
| UNPROVEN HYPOTHESES | Sender encoder/pacer queue or timestamp behavior; sender response to client feedback; selective network/Wi-Fi/driver queue | Different mechanisms produce the same available receive timestamps | Require the same packet at a second, named observation point before choosing a media correction |
| NOT SUPPORTED AS PRIMARY ONSET CAUSE | Multi-second RTC/reorder/AU queue residence, decoder-only delay, RTP wrap, SCTP retransmission storm, initial REMB cuts or catch-up request | Inconsistent with same-packet timings and chronology | Do not reapply those attempted fixes as the root correction |
| CONTRIBUTING DEFECT in the offline analyzer, corrected | External correlator previously required uniqueness only on the external side and omitted track generations/raw dequeue validation | Reused packet identities could be assigned across sessions; risk of a false inference | Require uniqueness on both sides and a matching raw dequeue; preserve generations, missing counts, clock-skew caveat and INCONCLUSIVE outcomes; regression tested |

No Vita source, media behavior, Home Refresh behavior, workflow or package configuration changes were made in this continuation. Changes are offline analysis, correlation tests and this evidence record. The existing diagnostic VPK remains the one authorized package. There is no corrected release candidate.

## Why the remaining measurement is genuinely external

The returned timestamps constrain dequeue D and bound socket residence K, but do not observe the earlier sender interval S or network/driver interval N. After accounting for known downstream residence, the observation constrains S+N+K, not S and N individually. A growing S with fixed N and a fixed S with growing, video-selective N can produce exactly the same complete device bundle, including timely audio/control and an empty socket immediately before receipt.

The new controlled test uses the **same saved/exported production-recorder fixture** with two synthetic wire histories. In one, the 2 s growth is already present at the external point and external→dequeue growth is zero. In the other, media is current there and external→dequeue grows 2 s. Device-only analysis is identical. The correlated external observation distinguishes the histories. Additional tests retain ambiguity for repeated identities across resets, duplicated observer packets, missing raw matches and a clock step affecting video/audio. These are constructed identifiability tests, not Xbox stream replay or evidence of an actual wire timestamp.

Source inspection cannot recover an event that was not recorded. Simulating either earlier history cannot tell which occurred on this console/network. This environment has no access to the user's Xbox, LAN capture interface, Vita driver trace, or kernel packet-arrival facility. The approved Vita API has no established, tested arrival-timestamp contract that would provide this missing point. More device-only dequeuing probes would repeat the same measurement.

The **single missing measurement** is the timestamp of those same RTP packet identities at a known point before the Vita receive stack, collected simultaneously with the existing diagnostic. Prefer a mirror of an already-wired Xbox's egress. An existing AP/router LAN-ingress capture is the fallback and has a different boundary: growth before that point still includes the Xbox-to-AP segment. Neither can alone distinguish Vita radio from driver delay. No new firmware experiment or media setting is needed.

## Minimum practical collection, using the same VPK

The precise interface depends on the user's router/AP and whether the Xbox is wired. A normal laptop on the same switched LAN is not automatically an observation point for these packets. First establish an existing capture/mirror facility that sees this traffic, including hardware-offloaded traffic. Do not move the Xbox to another transport merely to make capture convenient; that changes the failing setup.

1. Keep the returned bundle. With GreenVita closed, use VitaShell to rename `ux0:data/green-vita-540-test/latency-DIAG03-8/` to `latency-DIAG03-8-first/`. The original capture is deliberately immutable; this step, then relaunch, lets the **same VPK** record another incident. Do not delete the existing evidence or refresh/reconnect mid-capture.
2. On the established observer, begin a bounded classic-PCAP capture immediately before the normal Home session:

   ```sh
   tcpdump -i <observed-interface> -s 128 -c 200000 -w greenvita-observer.pcap 'udp and host <Vita-IP>' 2>greenvita-observer.txt
   ```

   Stop with Ctrl-C after the device collection. The packet cap bounds PCAP size to approximately 28.8 MB and preserves the beginning; reaching it early is incomplete evidence. The 128-byte snap length includes clear identities and possibly an encrypted payload prefix, not decoded game content. Retain stderr's received/dropped counts, the capture point/interface, and any known capture-clock adjustment. PCAPNG can be converted offline by the engineer; no user counter interpretation is needed.
3. Play until **Capture complete**, at most three minutes of video (allow the post-trigger ten seconds if collecting). Hold SELECT, choose **Exit stream (keep game running)**, wait for **Capture saved**, and return the four files in the newly created `ux0:data/green-vita-540-test/latency-DIAG03-8/` plus the observer PCAP/stderr. No screenshots, recording, refresh or acceptance matrix.

Analysis commands:

```sh
python3 tools/analyze_diagnostic.py bundle.zip
python3 tools/correlate_observer.py bundle.zip greenvita-observer.pcap --point 'actual physical capture point' --capture-drops 0
```

The last argument is **only** supplied if tcpdump actually reported zero; otherwise use its real number or omit it for unknown. The correlator joins authenticated packet identities, checks the original raw dequeue record and rejects ambiguous cross-session identities. Its per-second report separates observer/media displacement from observer→dequeue displacement. Constant clock offset cancels; relative clock skew or steps do not, so concurrent audio and capture-clock health remain mandatory interpretation checks. No recurrence means NOT REPRODUCED DURING CAPTURE, not proof of a fix.

A measured increase already at immediate Xbox wire egress targets encoder/pacer/media-clock/feedback behavior before egress. An increase added after that point targets the intervening path. That distinction is necessary before another architectural correction can be selected on evidence.

## Verification

The new offline tests exercise saved capture → delayed export → analysis → synthetic external correlation, not just counters. The full host audit passed all 28 command gates: 294 Rust test executions (shared production cases execute in more than one harness), strict host Clippy across 11 harnesses, and 49 Python tests after the final focused diagnostic rerun (20 capture/correlation, 18 latency analysis, six link-layout and five contracts). The authenticated two-peer transport and 30-virtual-minute repeated-outage/feedback stress tests passed. These tests do not emulate Xbox encoding, its congestion controller, the Wi-Fi driver or sustained Vita playback. Logs are under `verification/diag03-return/` in the working checkout; that generated directory is not source. Existing native success for the unchanged device source is workflow 36299664334 (DIAG03-8), and the prior analyzer follow-up's host/native gates completed successfully in workflow 36300311123. No new package is requested or produced by this analysis commit. Hardware sustained-playback acceptance remains failed/open.
