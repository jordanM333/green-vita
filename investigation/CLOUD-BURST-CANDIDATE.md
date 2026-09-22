# RX Test 36: Cloud recovery and feature foundations

## Observation, not a claimed device fix

Jordan clarified that RX 35.2 was nearly flawless for Home, including walking away. Cloud still hangs at a seemingly patterned interval. Preserve 35.2 as the Home rollback baseline. The new Cloud excerpt spans 193.999197–198.286103 s. It establishes one 550.632 ms fresh-picture gap, not a recurrence period. RTP sequence loss here is a receiver observation; the trace cannot locate its origin on the network, server, or local receive path.

## Evidence from the supplied Cloud excerpt

| Elapsed (s) | Event | Interpretation |
|---|---|---|
| 194.468171 | GPU done for RTP 4184280256 | Last fresh picture before the freeze |
| 194.552304–194.552311 | AU abandon, recovery begin, reason 0 | MissingSequence triggers an IDR wait |
| 194.685648–194.685975 | IDR 4184301226 complete and submitted | First recovery wait was 133 ms |
| 194.735410–194.735651 | Four complete AUs | Burst arrives in 241 microseconds while decode/poll is busy |
| 194.735667–194.735670 | Recovery begin, reason 3 | QueueFull invalidates the first recovery epoch |
| 194.739344–194.739364 | Three generation drops | Already queued dependent pictures discarded correctly after damage |
| 194.854867 | Next keyframe request | Existing 300 ms request gate shapes recovery timing |
| 194.967185 | Second IDR admitted | Second recovery wait was 231 ms; old epochs still drain |
| 195.018803 | Fresh IDR reaches GPU | 550.632 ms since last fresh picture; frame itself is 95.929 ms old |

The earlier 65.896 ms AU-arrival gap also delivers a burst. Generations 7850–7852 complete while the UI does not acquire a texture until 7853. Inspection confirms that the previous surface guard held the renderer's mutex throughout hardware calls. The trace alone cannot allocate every microsecond to lock contention; the controlled regression reproduces the blocking directly.

## Changes

- Exclusive surface leases release the ownership mutex before hardware calls. Displayed, pending and decoding surfaces stay distinct with three buffers. Two-buffer fallback withdraws a pending picture before reuse, even if decode returns no picture. Surface destruction closes admission and waits for the active lease before freeing CDRAM.
- Six compressed input slots absorb the observed four-AU burst plus headroom. A separate 4 MiB aggregate queued-payload cap bounds memory; the 50 ms dequeue deadline remains. No arbitrary reference-frame replacement or latency reset was introduced.
- During a queued burst, at most two input calls may run between output polls. Ordinary single-input servicing and independent old-epoch draining remain. Stop is checked between calls.
- `pipeline-incidents.csv` retains 1,024 recovery/gap events independently of the 4,096-row verbose tail. It records AU delivery and fresh GPU-picture gaps over 50 ms, recovery causes and waits, and keyframe requests. It rotates with the other diagnostics and writes only on stream stop.
- Catalog: local persisted Favorites, a most-recent-first Recently played list (recorded after decoded video appears, not on failed launch), section picker, favorite action and search within each section. Stable game IDs survive reorder and unavailable titles. Recently added has an explicit provider-order contract, disabled in the picker until connected; release date is never substituted for catalog-add date.
- Microphone: session-scoped, default-muted capability state, capture tickets and a final send-admission gate. Muting or losing capability invalidates pending capture tickets. Pause menu honestly shows unavailable; no capture/encoder/outbound RTP or party-chat support is claimed in this candidate.

## Validation

- On unmodified RX35.2 production code, the blocked-hardware renderer test times out and the four-AU admission test rejects the fourth AU. Both pass against this candidate.
- Production worker/decoder/texture code runs against a controlled SDK FIFO: 26 tests pass, including old epochs, PTS identity, independent output drain, ownership during decode, teardown, byte cap, unchanged 50 ms deadline, and incident retention. This is not a hardware emulator or a device timing measurement.
- Seven foundation tests cover stable-ID collections, MRU ordering and persistence, settings migration, and mic mute/disconnect behavior. Full existing CI gates and Vita compilation are required before package delivery.

## Device check and remaining work

Use Cloud first and exit normally after a hang to save diagnostics. Compare `pipeline-incidents.csv`, `pipeline-trace.csv`, `pipeline-history.txt`, and `pipeline-status.txt`; repeat on Home to check the shared worker changes. A hardware result is still required. The original missing-packet event is not eliminated by this patch, and periodicity is still unproven. Do not replace sustained acceptance with the controlled host test.

Mic completion needs Vita capture, a bounded mono encoder path, an actually negotiated outbound audio sender, and provider voice signaling tested with a friend. All final packet admission must use the mute gate. Recently added needs an authoritative Cloud collection feed mapped to playable stable game IDs. Favorites/history currently remain local to this Vita and are not Xbox-account synced. This is catalog groundwork, not a completed website-style shelves redesign.
