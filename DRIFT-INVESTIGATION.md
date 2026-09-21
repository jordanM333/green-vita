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
