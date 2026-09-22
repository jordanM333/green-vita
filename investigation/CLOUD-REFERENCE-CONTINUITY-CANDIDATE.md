# RX37: Cloud reference continuity and work-conserving decoder service

RX36.3 failed Jordan's Cloud test: "absolute laggy mess." It is not an accepted
Cloud build. RX35.2 remains the reported good Home rollback. These are changes
to a candidate, not a claim that host tests establish Vita playability.

## Supplied evidence

The incident excerpt spans 4.397-78.247 seconds; the detailed tail covers
72.222-78.316 seconds. There are ten QueueFull-triggered recoveries, including
four in 1.212 seconds (75.908432, 76.210977, 76.850846, 77.119737).

| Time (s) | Observation | Meaning |
|---|---|---|
| 60.456706 | `age_drop` for RTP3024666763, 52,010us queued | An intact reference was discarded by a 50ms latency policy |
| 60.460909-62.292077 | Recovery wait 1,831ms | Age policy created a new IDR dependency |
| 62.401612 | `new_picture_gap_us=1946296` | Actual fresh-picture gap was 1.946s |
| 72.372807-74.080358 | Missing-sequence recovery, 1,707ms | Packet damage is a separate remaining cause |
| 76.156993-76.163914 | Eight consecutive completed AUs | Larger burst than the previous four-AU example |
| 76.210977 | QueueFull, six AUs invalidated | Recovery re-entered after successfully admitting an IDR |
| 76.258571-76.602624 | RTP advances 105,030 ticks in 344,053us | 1.167s of media delivered in 0.344s: catch-up bursts, not steady 60fps arrivals |

During early-tail freezes, ordinary present returns remain about 0.6-0.8ms and
GPU queue waits about 2.6ms. Later rendering costs rise, but cannot explain
the 1-2s IDR waits. Numerous output-only decoder calls consume up to roughly
8ms while complete AUs await input service. Unretired PTS metadata permits an
output probe; it does not establish firmware readiness.

## Implemented correction

- Keep intact compressed references through temporary queue-age pressure.
  The old 50ms deadline is diagnostic only; it no longer invalidates an epoch.
  Genuine packet damage, input decode errors and hard resource exhaustion still
  require recovery. No DecodeStop, latency-triggered decoder recreation,
  reconnection, or arbitrary P-frame skipping is added.
- Prefer queued input over speculative output-only calls. Input decode can
  produce a picture itself. Idle gaps still drain independently. At eight
  unretired PTS entries, poll between inputs to service accumulated output debt;
  this is a scheduling watermark, not a claim of firmware queue capacity.
- Retain the 4MiB queued-payload bound and use a 32-AU hard count bound so tiny
  AUs cannot grow memory without limit. This admits short catch-up bursts; it
  is not a target delay. In-flight payload and decoder memory are additional,
  as before. Under sustained overload this guard can still cause recovery.
- Preserve newest-decoded-picture replacement and RX36's disjoint surface
  leases. No change to the 1280x720/refs1 decoder or RGB565 960x544 output.
- Trace queue residence, pressure, hard count rejection, pending metadata,
  AU byte sizes, video packet delivery gaps, released sequence holes and slow
  RTC pumps. Packet delivery times are application observations, not NIC
  timestamps. Recorder capacity and post-stop-only file writing are unchanged.

Microphone and catalog foundations are retained unchanged. Mic capture/uplink
and provider-backed Recently Added remain unavailable, not presented as working.

## Regression checks

The real production worker/decoder adapter/surface ownership runs with fake SDK
functions. The fake is a controlled FIFO, not a Vita emulator.

- Eight-frame burst admission and queued-input priority pass in RX37; the same
  test fails on RX36's six-AU queue. Increasing that old queue alone still fails
  the call-order assertion: the old worker polls after two input calls.
- An intact reference queued behind a 60ms hardware gate is decoded in RX37;
  RX36 never submits that second reference and the test fails.
- 32 withheld input outputs exercise output-debt service under sustained input;
  all identities retire once, without decoder recreation.
- Byte and count guards, independent idle draining, old-epoch rejection,
  unsupported polling, displayed-pixel ownership, teardown, PTS matching and
  frame wakeups remain covered.
- A separate deterministic scheduling model uses 27 AU completion times copied
  from this trace. With explicitly synthetic readiness/service costs, RX36's
  queue rejects six AUs. Enlarging the queue alone reaches 12 queued AUs and
  146.658ms maximum wait; input-priority scheduling reaches eight and 61.207ms.
  These numbers are model comparisons, NOT measured candidate latency. Packet
  loss and actual H.264/firmware decoding are outside that model.

Local decoder/shared suite: 30 passing tests. Offline analysis: 16 passing tests.
The local broader RTP build encountered a host Rust proc-macro linker failure.
The independent GitHub test job passed all gates, including 13 RTP-order and
30 decoder/shared tests, 109 test executions overall (shared tests run in more
than one suite). Workflow: https://github.com/jordanM333/green-vita/actions/runs/35678997960.
Vita release packaging passed as RX Test 37.4. The downloaded artifact and VPK
checksums match CI, ZIP integrity passes, and the executable embeds runtime
commit `70ccd93aee1c99bb8550344c264b0b6413ece1d9` and build `37.4`.
Only `eboot.bin` differs in package contents from RX35.2; title ID remains
`GRNVTEST1`. Exact provenance is in `CANDIDATE-37.4-BUILD.json`.

## Device acceptance still required

The trace proves local amplification of burst/age pressure; it does not locate
the underlying missing RTP/FU fragments. RX37 cannot recreate lost packets.
Test Cloud beyond the previous 38-78s failure window, then Home for regression.
Check that queue pressure drains without recurring QueueFull/age recoveries and
that output-PTS residence does not grow over time. Long freezes or accumulating
control delay still fail acceptance even if host tests and packaging pass.
