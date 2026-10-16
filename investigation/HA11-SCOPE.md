# HA11: raise the video ceiling from 3 to 4 Mbps

The user reported stutter in a busy home-stream scene on HA10-26. HA11 is a
hypothesis until a physical Vita confirms it. Cloud sessions have still not
been captured with the latency folder. Rollback source is 24da6bb (HA10-26).

## Evidence (HA10-26 pipeline-history.txt, 232 s)

Seven lag episodes (about 37-50, 84-87, 94-97, 113-122, 133-141, 149-153 and
183-192 s) share one pattern:

| | Calm play | Lag episode |
| --- | --- | --- |
| Wire RX RTP | 2.0-2.7 Mbps | 3.2-3.45 Mbps |
| Live edge ingress | 2-15 ms | 250-1,100 ms |
| V-A (SR offset) | ~0 ms | up to 1,098 ms |
| Ingress V media/wall | ~1.0 s/s | 0.3-0.8 s/s, then 1.2-1.5 s/s catch-up |
| Shown fps | 60 | 19-50, then 75-85 |

Across the session: Order lost 0, NACK queued 587 with no failures, no decoder
errors, decode 3-4 ms, UI 5 ms. Audio stayed on time. A 1 s backlog at
3.3 Mbps (~400 KB) cannot sit in the Vita's 124,800-byte receive buffer
without loss, so video was queued upstream (Xbox pacer or access point).

Hypothesis: busy scenes need more than the 3 Mbps the Vita allows, and the
Xbox queues video behind the cap. The access point buffering at that rate is
the alternative; HA11 tells them apart.

## Changes

| Change | Where |
| --- | --- |
| `VIDEO_CEILING_BPS` 3,000,000 -> 4,000,000: SDP `b=AS:4000`/`b=TIAS:4000000` on the stream and chat offers, and the fixed REMB target. | bandwidth.rs |
| Tests follow the new ceiling. The sender-backlog model keeps HA07 and HA09-HA10 at 3 Mbps (their heavy-scene lag stays covered) and adds an HA11 case: the same gameplay at 4 Mbps never falls 300 ms behind. | audit-contracts, rtc-transport, rtp-order |
| Build identity HA11. | diagnostic.rs, final-build.yml, build_provenance.py |

## Reading the result on hardware

* Busy scenes above 3.3 Mbps with Live edge ingress in tens of ms: the cap was
  the cause.
* Lag returns at about the same rate: the Wi-Fi link is the limit; lower
  resolution is the next lever.
* `Order ... lost` above 0 or NACK failures: 4 Mbps is too much for the link;
  return to 3 Mbps.

## Call-outs: constrained settings that HA11 changes

* **Bitrate and SDP ceilings:** 3 -> 4 Mbps (user approved).

Unchanged: the `maxBitrateKbps` 2000 hint, resolution, receive budgets, thread
priorities, audio policy, the microphone, the DISPLAY01 and latency captures,
and Home sign-in handling.
