# RX Test 34: input starvation and receive congestion

RX Test 33 did not resolve the reported delay. The attached video identifies
RX Test 33 Home streaming and shows video decoding continuing during the delay.
It is not a packet trace and cannot identify the location of all remaining lag.

## Confirmed Test 33 regression

The 256-byte input and 128-byte presentation thresholds count unacknowledged
payload, not just unsent data. With 60 controller plus 60 presentation reports
per second and 100 ms RTT, normal in-flight input consumes the feedback limit.
The previous 5 ms one-way tests missed this starvation because they asserted
only the age of delivered messages and maximum outstanding bytes.

A 29-second two-endpoint SCTP regression now reproduces:

| Healthy 100 ms RTT | RX Test 33 policy | Corrected admission |
|---|---:|---:|
| All reports delivered | 1,378 | 3,624 |
| Presentation reports admitted | 1 | 1,812 |
| Maximum delivered report age | 50 ms | 50 ms |
| Maximum gap between delivered reports | 64 ms | 16 ms |

The corrected path reserves actual SCTP congestion/receive capacity, accounting
for pending data on the association and WebRTC ingress on all its channels.
The small application limits apply only to unsent ingress. The pump flushes
admitted reports before processing a timeout that could reduce the send window.
It still honors SCTP congestion control and retains current unsent controller
state. Under intermittent loss the same test delivers 3,576 reports, admits
1,812 presentation reports, has 50 ms maximum delivered age and a 48 ms maximum
delivery gap. The full ICE/DTLS/SCTP API test verifies ingress rejection, continued
sending while ACKs are absent, actual-window exhaustion and resumed fresh input.
This demonstrates starvation and its correction, not Xbox's internal response.

## Receive congestion response

The previous REMB request stays at 2 Mbps even while RTP arrival delay grows.
The negotiated REMB path now reduces its target after two 200 ms windows with
at least 100 ms added RTP arrival delay. Decreases are limited to once a second;
the requested floor is 500 kbps. It leaves headroom below measured video payload
throughput and raises the target by 100 kbps only after five seconds with less
than 40 ms added delay. The maximum remains 2 Mbps. Transient jitter does not
trigger a reduction. Diagnostics expose the target, delay and reduction count.

A virtual 2-to-1 Mbps bottleneck test with 100 ms feedback delay shows the fixed
request accumulating over 50 seconds of backlog in one minute; the adaptive
policy ends below 100 ms. This is a controller simulation, not evidence that
Xbox obeys every REMB request. The actual Xbox response remains device-dependent.

No automatic reconnect or latency-based decoder flush is added. Lower bitrate
may temporarily reduce image quality. Remaining uncertainty: the recording and
old logs do not establish that these changes remove every source of end-to-end
delay on the user's Xbox/Vita network. Device playback is required to establish
that outcome; compilation and simulated tests cannot do so.

---

# RX Test 31 drift: evidence and verification

## What the supplied logs establish

The fresh-process session ends at 63.082s; the other pasted session follows its
reconnect (its PTS counter starts at 2967). Some metrics are process-wide, while
RTP/SR counters restart per connection. Read them as two sessions, not one series.

| Session / elapsed | Decoded FPS | Decoder frame age, average/max | Video payload | Video RTP relative growth |
|---|---:|---:|---:|---:|
| Fresh / 59.077s | 61 | 85/106ms | 1927kbps | 1ms |
| Fresh / 61.079s | 7 | 380/483ms | 310kbps | 914ms |
| Fresh / 63.082s | 0 | No output | 194kbps | 2466ms |
| Reconnect / 52.280s | 51 | 157/317ms | 1362kbps | 154ms |
| Reconnect / 55.283s | 2 | 2233/2324ms | 47kbps | 2437ms |

At collapse, receive polling continues about 420 times/second, no pass exhausts
its budget, the decode queue is empty and a hardware decode still takes about
2ms. The application is receiving too few advancing video frames. Empty local
queues do not establish that the received content is current. Sender report
counts also stop advancing while relative RTP arrival delay grows.

The data does not locate the upstream delay between Xbox, network and hidden
transport queues. ICE RTT is periodically sampled and can be stale during a
collapse. Audio-relative history jumps are not proof of equivalent audio lag.
`DCbuf` was a high flag/event count, not queued bytes. Server 3840x2160 is session
metadata: the bitstream and hardware decoder are both 1280x720.

## Confirmed regressions and reproduced mechanism

1. Build 31 used 100ms decoder residence as a latency alarm even though healthy
   samples commonly take 85–120ms from receive to decoder output. It could flush
   repeatedly and escalate to a full reconnect. The parent containment change
   removes that automatic policy; manual Refresh stream and real damage recovery
   remain. A healthy post-reconnect interval is not proof of a permanent fix.
2. Zero SCTP retransmissions does **not** prevent unsent reports waiting behind
   its congestion window. A two-endpoint test of the exact shipped
   `rtc-sctp=0.20.0-rc.2` reproduced **2013ms-old input/feedback** and **16254B**
   outstanding during a two-second reverse-path outage. After acknowledgements
   recover it eventually clears; this is not a reproduction of permanent drift.

## Correction and local verification

Admission now measures both WebRTC ingress and SCTP queued/unacknowledged bytes.
Controller/pointer payload has a 256B ceiling; presentation feedback has a 128B
ceiling on the same channel so input retains headroom. Dropped presentation
reports are replaced by future presentations. Rejected controller state stays
in a single replaceable slot; a Guide release is retried even if UI sampling is
temporarily suppressed. Control/chat/message channels are unchanged.

The new `Input transport sampled` history row exposes the last admission sample,
process maximum, deferrals and API errors. It is not a delivery-age measurement.
Existing frameReport failure counts include intentionally deferred feedback.

Six tests pass in `tests/rtc-transport`:

| Scenario | Maximum delivered report age | Maximum outstanding payload |
|---|---:|---:|
| Two-minute steady baseline, 5ms each direction | 5ms | 129B |
| Baseline, repeated two-second ACK outages | 2013ms | 16254B |
| Bounded, same ACK outages | 5ms | 215B |
| Bounded, repeated two-second bidirectional outages | 5ms | 215B |

The other two tests exercise full WebRTC admission (including writes before
poll_write and acknowledgement accounting) and unsent press/release replacement.
RTC dependency versions in the host-test lockfile match the Vita lockfile.
Host tests use the host registry versions of ring/rtc-shared; the device build
retains its existing Vita patches.

Run `cargo test --locked --target x86_64-unknown-linux-gnu -- --nocapture` from
`tests/rtc-transport`. The baseline test must reproduce the large delay and
queue; the bounded cases must pass under the same virtual network schedule.

## What is not verified yet

The SCTP mechanism is independently reproducible, but the old logs do not expose
its byte queue. They cannot prove it caused the incoming-video collapse or that
Xbox's encoder reacts to stale frame reports in the hypothesized way. A byte cap
does not impose a delivery deadline, remove delay elsewhere in the network, or
prevent another SCTP stream from occupying the association's congestion window.

The acceptance gate for calling the home-stream drift fixed is an on-device
session of at least 30 minutes with active play, an idle/return interval, and
temporary congestion followed by recovery. Keep automatic latency reconnects
disabled. Pass only if controller-to-visible-response delay stays near its
initial baseline after recovery, no reconnect is needed, incoming video does not
remain behind, and button releases recover correctly. Compare visible response
with the Xbox display (or a timed controller/video recording); the local stage
timers alone cannot verify end-to-end latency. A compiler/test pass does not
substitute for this gate.
