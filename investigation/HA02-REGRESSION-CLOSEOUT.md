# HA02 focused regression correction

This candidate is for hardware acceptance. The original progressive-latency
issue remains open. Client defects below are reproduced; the exact device
trigger and the original Home service error code are not established by the
available photograph. Do not label either physical symptom conclusively fixed
until the returned device evidence supports it.

## Starting artifact and scope

* Application source: `fc1d008dc0e5279c87c0db2bb24dc5b3fda18d4a` on
  `architecture-review-20260926`, tree `45d887f1126781e34f3754bb7dffbe937b6fe245`.
* HA1 packaging workflow `36375302601`, packaging-only commit
  `8fb18057845326a988066666eee8c12a2afe23a9` on `hardware-acceptance-fc1d008`.
  That workflow checked out the exact application source above.
* Embedded `DIAG03-HA1`; VPK SHA-256
  `90c5311f6c248424815f2a9d22f69653591cabd7abcb3357806f7a9f79992a8f`.
* Last returned positive Home playback evidence: DIAG03-8,
  `af53e01bb3a16cba3add48023396fb8fa0849169`. Home creation, credentials,
  provisioning and cleanup code have no difference between that source and HA1.
  Cloud's last positive observation in this conversation precedes the live-edge
  change; a specific more recent Cloud log is not available to establish a
  different exact last-good source.
* No bitrate, resolution, receive budget, queue capacity, thread priority,
  reorder horizon, keyframe frequency, or media freshness threshold was tuned.
  No console-power/game-termination action or session sweep was added.

## Findings and corrections

| Finding | Evidence and confidence | Correction |
|---|---|---|
| Optional sender-report calibration could permanently invalidate healthy playback | CONFIRMED client defect. Production policy test with continuous 60 Hz RTP and a one-frame SR inconsistency fails at frame 60 / one second on HA1. `valid=false` blocks presentation immediately; the next poll enters `ClockUncertain`, whose IDR path cannot restore that clock. Not proof that this particular report occurred on the user's Cloud run. | Reject/count the inconsistent SR pair, retain the independent RTP/monotonic timeline. No threshold increase or stale-clock reset. |
| Startup setup traffic was treated as established playback | CONFIRMED client defect. A setup packet followed by one second of startup waiting enters `AwaitingKeyframe` before any picture. A different initial media timestamp can also invalidate the provisional mapping. | Keep `Unmeasured` until a matched, valid-epoch picture completes rendering. Establish once from that picture's original dequeue time. Local 240 ms admission remains enforced before establishment. |
| Fast arrival of advancing media could invalidate the clock | CONFIRMED policy defect. Advancing 600 ms of RTP in a 20 ms catch-up burst invalidated the mapping instead of improving the earliest observed baseline. | Accept the earlier baseline; later arrivals still cannot move it later. Subsequent genuine staleness still fails. |
| Setup packets disabled the initial picture watchdog | CONFIRMED wiring defect. A non-picture packet set `received_packet`, disabling initial keyframe requests forever. | Watch for produced pictures, not merely packets; retain existing request interval. |
| Home discarded its service failure identity | CONFIRMED diagnostic/error-state defect. The displayed string can only be raised after a successful state HTTP/JSON response containing `Failed` or `Error`. Error details were not deserialized. | Preserve bounded service code, detailed state and handshake phase; classify terminal failure separately from retriable transport polling errors. Save a per-mode bounded startup journal independently of packet capture. |
| Repeated ReadyToConnect could resend an accepted connect POST | CONFIRMED lifecycle defect, NOT confirmed cause of the photographed service failure. | Remember acceptance in the owned Stream (including polling clones). Continue state GETs without repeating a successful handshake. A newly created session gets a new handshake. |

The photograph proves console selection and `/play` returned a usable session
path and a later state response was `Failed`/`Error`. It does not prove the
console accepted startup, which provisioning transition preceded failure,
whether `/connect` had succeeded, or which service code was returned. No new
HA1 packet/history bundle was available in the supplied files. Old DIAG03-8
timing data cannot supply those missing values. It would be incorrect to claim
the Home server rejection itself has been identified and corrected.

Audio has its own RTP clock and no video IDR/presentation gate. A video-only
invalid clock therefore explains the *possibility* of black video with healthy
audio without requiring shared transport failure. SR rejection is now also
nonfatal for audio; true audio media delay remains bounded.

RFC 3550 section 6.4.1 defines SR RTP/NTP pairing separately from media packet
timestamps: https://www.rfc-editor.org/rfc/rfc3550#section-6.4.1. An inconsistent
calibration sample does not independently prove old media. The correction does
not assume Xbox's actual SR error without a matching capture.

## State and ownership review

`Unmeasured` allows locally current decode/presentation while setup completes;
it cannot claim a known capture age. First matched presentation establishes a
relative baseline. The initial picture watchdog stays active during setup.
After establishment, stale detection, epoch invalidation, obsolete dependency
rejection, matching SPS/PPS plus current IDR, bounded recovery requests and
matched current presentation are still required. `establish` refuses to reset
an established/recovering clock. Empty queues and IDRs alone never prove recovery.

`ClockUncertain` remains explicit unavailability for genuine post-startup RTP
discontinuities; the UI names exit/start as the valid failure path. Waiting for
a fresh IDR/picture has a bounded request budget and an explicit exit path; no
periodic purge or automatic session destruction was introduced. A sender that
never supplies current decodable media cannot be forced to recover by this client.

Home REST provisioning happens before `RtcSession::new`, the recorder and the
video clock. The preserved-capture early return changes recorder state only;
it does not change session credentials, REST state or media ownership. Each new
media receiver owns new clocks/output. Cloud stop remains owned-session DELETE;
Home stop remains non-destructive detach; Home refresh remains media-only.

## Verification and limits

New integration test runs real RTP assembly, parameter parsing, queue,
decoder adapter, PTS matching and output ownership for 180 pictures over more
than three wall-clock seconds. Setup precedes media by two seconds, RTP wraps,
and a mismatched SR arrives at frame 60. Native decoder calls and the display
callback are host substitutes; this is not a Vita performance claim.

The 5,003-observation replay still quarantines at 338 ms relative divergence;
30-minute 1.595-second stale-plateau model still rejects old video, and current
IDR plus matching presentation conditionally restores LIVE. The timing-only
replay explicitly seeds an already-playing baseline from the retained window;
it cannot infer payloads or a startup display event. Six-hour oscillator tests,
loss/reorder/backpressure, native host Opus, voice on/off, lifecycle and static
checks remain in the full quality gate.

Home tests run actual Stream methods with scripted HTTP responses for
Provisioning → repeated ReadyToConnect → Provisioned → SDP, Home stop/start,
Cloud → Home and terminal failure. These prove client request ordering and
ownership, not actual console discovery/authentication or Xbox availability.

Startup journals contain at most 32 bounded lifecycle entries per mode and
are written only at REST state/handshake changes, never per packet. Packet
capture remains bounded, asynchronous from the receive path, and preserves
the first incident. New establishment/ignored-SR transitions are retained in
the trace. Final exact-source CI results and VPK identity/hash are supplied with
the artifact; native test compilation is not device test execution.

Hardware acceptance must determine whether these corrections restore the
reported Cloud session and Home service establishment. If Home still rejects
startup, `startup-<build>-home.txt` now gives its service code and handshake
phase. Do not call that possibility an already-proven Home fix.
