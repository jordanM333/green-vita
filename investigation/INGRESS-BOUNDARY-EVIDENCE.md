# Ingress timing contract and limits of the AR01 device evidence

Parent source: `0d5276d1fd9bc9b4da839d6ea3e442e88e719094`.
This change corrects ingress timestamp ownership and supplies missing boundary measurements. It does **not** establish a correction for the reported progressive latency. No release-candidate packaging is authorized by this evidence.

## Inflection, from the supplied device history

Both histories identify AR01-4, `68dc5032c6751a1e2ce0282c6eb27df0a61ae6e3`, Home, microphone off. All numbers below are observations from the unmodified supplied files. They are not results of the new instrumentation.

| Current session elapsed | Relative video delay | Relative audio delay | Video SR offset | Video assembly avg/max | AU waiting avg/max | Completed RTC pump max |
|---|---:|---:|---:|---:|---:|---:|
| 91.125 s | 4 ms | 7 ms | 2689 ms | 2/25 ms | 0/6 ms | 4 ms |
| 92.127 s | 1 ms | 15 ms | 2687 ms | 10/105 ms | 42/134 ms | 4 ms |
| 93.127 s | 350 ms | 1 ms | 3035 ms | 18/49 ms | 6/51 ms | 4 ms |
| 94.128 s | 862 ms | 1 ms | 3547 ms | 28/50 ms | 0/2 ms | 4 ms |
| 95.129 s | 1367 ms | 11 ms | 4053 ms | 29/54 ms | 0/7 ms | 5 ms |

Between 92.127 and 95.129 s, 3.002 s of wall time elapses while the latest received video media timeline advances approximately 1.636 s: a deficit of 1.366 s, or about 455 ms/s. These are estimates from 1 Hz samples; the exact packet times at this inflection have rolled out of the event ring. A constant baseline clock offset cannot create this slope. SR offset independently changes by exactly 1366 ms over the same samples; audio does not follow that change. Absolute SR offset is NOT one-way latency because clock synchronization is unverified.

During those windows, RTP processing work is 196, 207, 211, 223 ms, for 267, 265, 306, 304 packets respectively (audio and video combined). These are serviced packet counts, not network arrival counts. The receive worker completes 437, 433, 416, 424 passes. The known 2 ms pass budget is hit 68, 74, 78, 83 times. Neither rate proves that a kernel queue is empty. The code formerly merged socket errors and readiness-cache WouldBlock into unclassified stops. There is no logged per-pass actual-empty event or kernel arrival timestamp.

The previous session independently changes from relative V/A = 3/5 ms at 17.148 s to 926/4 ms at 19.148 s. Assembly is 25/51 ms and decoder waiting 0/7 ms at 19.148 s; completed pump max is 3 ms. At the ends of the sessions, video relative delay is approximately two seconds. The current tail trace contains only 2.728 s; receive-to-submit maximum is 116.707 ms. It cannot reconstruct packet ingress at 92 s.

History SHA256:
- `pipeline-history(4).txt`: `15247bf707390590832855928ed343a6084995b8ea9531c6c2438fa13526f762` (144 samples).
- `pipeline-previous-history.txt`: `d8eebeefaf4e1fb7dbc76e1fd9d38d0df3d23cf459713c7a1ec938a331598e27` (217 samples).

## Boundary reconstruction

`RtcTransport::receive` tags UDP dequeue with Instant, runs the synchronous RTC handlers, and stops after at most 32 datagrams or a 2 ms processing budget. The session immediately drains all public RTC messages before entering reorder/assembly. RTP passes SRTP authentication and the interceptors directly; it does not pass through SCTP's reliable delivery queue. Every intermediate RTP message keeps the datagram Instant. The public poll API previously discarded it. The next stage then replaced it with Instant::now().

The enclosing measured pump is only a few milliseconds at the onset. Subject to its 1 ms display quantization and one-window reporting offset, a two-second residence cannot fit inside those synchronous calls. This source-plus-duration bound places the already-stale data at or before UDP dequeue. It is stronger than any queue-depth inference, but is not an actual historical kernel-ingress measurement. Video and audio use the same UDP socket; a continuously backed-up common FIFO by itself is a poor explanation for fresh audio alongside late video. That observation does not identify the Xbox encoder, sender pacer, access point, Vita network driver, or kernel as the cause.

| Boundary | Old evidence | New contract |
|---|---|---|
| Xbox capture/send and network arrival | Not recorded | Still requires an external packet timing observation; not invented from receiver counters |
| Kernel UDP -> application dequeue | UDP service bytes/packets, aggregate pass budget | Actual non-consuming MSG_PEEK empty observation, conservative residence bound, unknown/bounded sample counts, socket option capacity, errors and between-pass gap |
| UDP dequeue -> authenticated RTC delivery | Bounded indirectly by enclosing synchronous pump; timestamp lost at public API | Same datagram Instant retained by poll_read_with_timestamp; audio/video residence avg/max and independent dequeue-relative media clocks |
| RTC delivery -> reorder release | Held packet waits, but no separate handoff residence | Same-packet RTC delivery Instant preserved through reorder; measured residence avg/max |
| First ordered fragment -> AU completion | Conflated with earlier receive/reorder time | Independent assembly start and duration; h264_assembly_us trace event |
| First UDP fragment -> AU/decoder/output | Began at later RTC delivery | Earliest authenticated UDP dequeue Instant retained through AU, decode, presentation, audio age policy and frame feedback |
| Completed AU -> decoder submission | Existing queued_at/received_at and trace | Existing queue residence remains distinct from full packet age |

Independent relative clock minima are not subtracted to manufacture a residence value. Residence uses the two Instants for the same packet. Queue sizes, bitrate, resolution, keyframe policy, refresh behavior and thread scheduling are unchanged in this change.

## Real empty-socket bound

If an actual syscall establishes the UDP queue was empty at E, any subsequently dequeued datagram at D has queue residence at most D-E. The observation uses the timestamp **before** the empty-check syscall, giving a conservative bound even if descheduled during the check. This is not an estimate of the actual residence. A large bound does not prove large residence. Before the first verified empty observation, residence is unknown. A window with no received datagrams has zero bounded samples and cannot certify fresh media.

Tokio try_recv_from can return WouldBlock from its readiness cache without making a syscall. The diagnostic therefore peeks one byte directly only after the normal receive path returns WouldBlock. It never consumes data, treats zero-byte datagrams as data, and records errors separately. A successful peek means data is present at that later check; it can be a concurrent arrival and is **not** proof of a readiness bug.

Normal receiving remains inside Tokio/Mio. Replacing it with a raw syscall would bypass Mio's WouldBlock re-arm on Vita's poll selector, which would introduce a new scheduling defect. Audited host dependencies: Tokio 1.52.3, Mio 1.2.1, socket2 0.6.4. Vita SDK newlib source inspected at `vitasdk/newlib` commit `2e428297c0b6aefd830c5a75a7daa7e774562a42`: recvfrom maps a POSIX descriptor to sce_uid and calls sceNetRecvfrom; poll uses sceNetEpollWait. This upstream newlib revision is not asserted to equal the pinned SDK image's source revision. No raw POSIX descriptor is passed to a SceNet function.

## Issue register

| Classification | Evidence/mechanism | Correction and expected measurable change | Risk/verification |
|---|---|---|---|
| CONFIRMED MEASUREMENT DEFECT | Public RTC output discarded the original ingress Instant; video/audio restarted age at application delivery. A delayed RTC handoff could be made to look fresh. This alone cannot explain the observed two seconds inside a few-ms pump. | Preserve the tag, feed it into video/audio lifetime, independently measure RTC residence. A deliberately held old packet must retain its full age. | Changes existing age-based rejection/feedback to include previously hidden time. Actual encrypted two-peer regression, >2 s injected application hold, 30 virtual minutes, RTP/sequence wrap, H264 reorder-to-submission identity regression, existing audio age tests. |
| CONFIRMED MEASUREMENT DEFECT | Non-budget receive stops did not establish a real kernel empty observation. | Separate direct empty checks, pending-data checks, errors and scheduler gaps; prove only conservative bounds with explicit unknown/sample counts. | One small non-consuming syscall per readiness stop; Vita runtime overhead remains unmeasured. Local UDP tests verify no packet consumption, zero-byte datagrams, bounds and reset semantics. Normal Mio I/O remains intact. |
| CONFIRMED MEASUREMENT DEFECT | The label asm included reorder residence; changing ingress origin without relabeling would make it still more misleading. | socketToAU includes the whole receive-to-complete span; H264 ordered-to-AU measures the independent assembly interval. | Trace schema adds one event per completed AU; bounded trace rings retained. Parser tests keep absent measurements unknown. |
| NOT SUPPORTED AS THE TWO-SECOND MECHANISM | Current hardware assembly, reorder and decoder waiting measurements are much shorter than the video-relative growth already present before them. | Do not retune these queues in response to an upstream delay. | Hardware may have additional smaller downstream defects; this is not a declaration of overall health. |
| UNPROVEN HYPOTHESIS | Kernel/driver residence or scheduling before UDP dequeue. | New ingress contract can bound the socket contribution on an actual failing device. | Old history cannot supply missing kernel observations. Host OS networking cannot certify Vita behavior. |
| UNPROVEN HYPOTHESIS | Video-specific Xbox sender/encoder/pacer or network scheduling. | Requires actual source-to-receiver packet timing to distinguish mechanisms and feedback response. | The same received RTP progression can be produced by different pre-dequeue delays; no Xbox internal behavior is emulated by these tests. |

## Hard information boundary

For an identified RTP packet, the observed media-to-application displacement is the sum of capture/encode/send waiting, network/driver waiting, kernel residence, and RTC residence. The old logs supply only that sum and short downstream intervals. Moving a video-specific delay from a sender queue to a network/driver queue produces the same receiver packets, timestamps, SR offsets, audio progression and application counters. No deterministic receiver replay can select between those worlds: its input would already encode the answer.

The missing observation is a **time-correlated packet ingress trace during the real latency inflection, keyed by RTP identity, with a real socket-residence observation**. This is the single next boundary measurement needed to separate client receive accumulation from already-stale incoming media. If it locates the delay before client ingress, matching Xbox-egress timing is necessary to distinguish sender from network. Existing logs contain neither; repository captures contain no pcap/pcapng or complete raw RTP recording. The execution environment is x86_64 and has no connected Vita or Xbox transport. Native CI compiles the SDK target; it cannot run the console's kernel, wireless driver or Xbox stream. Instrumentation cannot retroactively produce the missing observation from the completed sessions.

This is the external blocker, not a claim that the application is fixed. The new probes are automatic and contain no media payloads or credentials. They are committed and checked without producing a VPK or asking for another gameplay trial. The explicit packaging gate remains closed.

## Verification scope

The local full host audit passed 270 Rust tests, 29 Python tests and strict Clippy for all eleven harnesses before final dependency pinning; focused UDP tests and Clippy also passed with socket2 pinned to the audited 0.6.4. CI reruns the full suite on the exact committed source, plus the vendored SCTP tests, Vita target/test compilation, strict target Clippy and native release compilation. Check the exact commit's workflow for those results; compilation is not device validation.

New sustained tests process 216,000 synthetic authenticated audio/video RTP packets over 30 virtual minutes and exercise a >2 s withheld public poll followed by bounded bursts. A separate 30-minute clock test injects recurring delay growth, catch-up bursts, duplicate/reordered timestamps and wrap. These are timing/transport tests, not a replay of an Xbox game or a Vita performance benchmark. The unchanged broader suite covers decoder overload, stale-frame rejection, dependency recovery, audio buffering, Home refresh ownership, repeated lifecycle transitions, voice and diagnostics modes.
