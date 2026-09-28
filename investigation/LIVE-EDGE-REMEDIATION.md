# Client live-edge remediation

Base/rollback: `8ea93088d2d1e8a0bb6cfff9eb43855ab5841adc`, branch
`architecture-review-20260926`. Continues DIAG03-8; does not restart the physical
upstream investigation or change Home Refresh ownership. No VPK is authorized
by this implementation commit; the workflow packaging marker is absent.

## Findings and scope

Question B has a demonstrated client defect. The former catch-up controller
explicitly ignored media delay when fewer than eight AUs were queued. Admission
and selection tested only time since UDP dequeue. Thus video already 1.595 s
late at that boundary could repeatedly pass admission with a nearly empty queue.
The surface could then redraw its uploaded texture indefinitely. These are
independent of the physical origin of the delay. An IDR does not prove freshness.

The earlier SCTP/local-queue work did not close this gap. Local residence is
useful evidence but is not media age; an empty queue is not a live-edge invariant.
The receiver bitrate controller also treated a *constant* stale plateau as
healthy enough to raise its ceiling. That is a confirmed feedback deficiency,
not proof that feedback caused the pre-socket onset.

Question A remains open: the measured pre-socket growth cannot distinguish
sender, network, Wi-Fi/driver, and earlier Vita receive-stack residence. No Mac
capture is a prerequisite for the client corrections here. No claim of restored
sustained Vita gameplay follows from passing these host tests.

## Invariant and clock model

For each active SSRC, let `E(r)` be the best observed monotonic delivery timeline
for RTP timestamp `r`. Unwrap modular timestamps and packet sequence numbers;
duplicate/reordered packets never move that timeline forward. A later dequeue
does not rebase `E`. Normal recovery, IDR receipt, a pause, shallow queues, and
diagnostic toggling cannot forgive accumulated lag. Only an explicit new stream
identity/lifecycle starts a new timeline. Empty probes are not media samples.
Queued packets must match both the active track and SSRC before any clock or
assembly observation; a prior source cannot initialize its replacement's clock.

Added media age at a boundary at time `t` is `max(0, t - E(r))`. This is a relative
lower-baseline measure, **not absolute capture age**. A session already delayed
at its first observation remains unidentifiable without another timing source.
UDP dequeue is still called dequeue, not network arrival. Existing timestamps
are preserved for packet correlation and rendered-frame feedback.

The existing local ceiling remains 240 ms. The ingress allowance is another
240 ms; the software presentation-admission ceiling is their sum, 480 ms. This
is a conservative safety ceiling, not a playout delay or a claim of acceptable
480 ms gameplay. Healthy captured dequeue-to-GPU residence was below 123 ms;
even the onset interval's maximum was below 197 ms. The new property depends
on enforcing a finite media deadline, not on increasing any buffer or tuning a
sender quality parameter. The final hardware UX budget remains to be validated.

Ingress age above 240 ms must persist for 120 ms (two maximum 60 ms repair
horizons); age above the entire 480 ms ceiling triggers immediately. Packet
silence/frozen timestamps also consume this same media deadline. Polling that
deadline is not a periodic purge. Short jitter and reordering within the budget
do not invalidate a reference chain. A source-clock discontinuity is explicitly
uncertain and fails closed instead of resetting its delay to zero.

SSRC-matched sender reports retain their original dequeue instants. RTP/NTP
increments validate the nominal rate; a minimum 30-second healthy SR interval
can calibrate relative oscillator rate within +/-1000 ppm without changing
phase. Calibration is frozen on a stale path. These assumptions do not prove
absolute one-way age: sufficiently slow correlated SR/media delivery drift can
remain indistinguishable from oscillator skew. No report or media observation
is silently treated as an external network timestamp. The recorded rapid
1.595-second divergence is far outside this skew allowance.

## Recovery and shared Home/Cloud behavior

1. Mark one incident, invalidate queued/in-flight decoder epochs, clear reorder
   and partial AU state, release queued reservations, and stop drawing old video.
2. Continue network draining and truthful RR/TWCC. Reject media beyond the
   deadline before reorder/AU/decode; recheck at later boundaries.
3. Request a fresh decodable point with the existing PLI backend. At most three
   requests per phase, using the existing 300 ms admission cooldown and bounded
   backoff; allow one further phase only after current timestamps return and the
   first budget was exhausted. No repeated purge, reconnect, or game termination.
4. Require an intact, current IDR with matching SPS/PPS. Cache only validated
   parameter sets, up to 128 KiB raw bytes, scoped to the source; revalidate PPS
   against current SPS and prepend required sets when sent separately. A malformed
   update invalidates cached assumptions. Existing H.264 fragment, size, and Vita
   resolution checks remain in force. Decoder errors still invalidate the chain.
5. Declare recovery only after an actual matched picture from the active epoch
   completes rendering within the media deadline. A queued IDR or an old render
   completion cannot declare LIVE. Every repeat draw rechecks age and epoch.
   Expiry also bypasses the scheduler's 250 ms unchanged-frame paint skip on the
   next input-loop pass (normally 4 ms); once black is drawn, idle GPU work stays
   suppressed. This is software scheduling, not a measured panel scanout bound.

If only stale data arrives, video becomes visibly unavailable with a recovery
message. This avoids ongoing stale gameplay but **is not successful recovery**.
PLI is a request, not a protocol guarantee of a fresh IDR or upstream purge.
Fresh, decodable media is a necessary condition for liveness.

Both Home and Cloud use this receiver/decoder/output contract. The policy is
independent of microphone state and diagnostics visibility. Home's explicit
media refresh remains a separate operation; session deletion/power/game commands
are not added to automatic recovery.

Audio uses an independent RTP timeline and a propagated media deadline through
Opus and PCM admission, including queued device audio and the entire new buffer.
It retains the existing 240 ms local limit and 120 ms device-queue trim limit.
Decode maintains Opus prediction even when stale PCM cannot play. Current audio
is not delayed to match obsolete video. Independent relative clocks do not prove
absolute A/V synchronization; that still needs valid shared sender mappings and
hardware acceptance. During video unavailability, audio and input remain active.

## Replay and tests

The committed address-free timing fixture is extracted from the supplied
`events.csv`, SHA256
`7a630fd5f37d656fc5d38940c018025ade2db236003bb0f3459897e8308dfa1b`.
`tools/extract_live_edge_fixture.py` verifies that hash and the single video/audio
stream identities. It retains all 5,003 nonempty RTP observations between
103 and 122 seconds: 4,070 video and 933 audio packets, including original RTP,
sequence, UDP dequeue and RTC delivery times. It does not interpolate the missing
98 seconds, reconstruct absent H.264 bytes, or simulate a real sender response.

The production policy replay rejects the delayed video while independently timed
audio remains admissible. Exact onset, rejected counts and bounded request counts
are printed by `returned_diag03_8_onset_blocks_stale_video_and_preserves_current_audio`.
Its audio baseline is the continuous retained window; its result is not the
same quantity as the earlier whole-session ~112 ms audio offset.

Additional behavioral gates cover the 1.595 s model with empty queues and a
30-minute stale plateau; conditional fresh-IDR recovery; lost/expired IDRs;
frozen timestamps and silence; wrap, duplicates, reordered packets, jitter and
valid timestamp pauses; six simulated hours at each of -500/0/+500 ppm; stale
SRs not rebasing recovery; real H.264 assembly/parameter identity; the production
worker rejecting pre-socket-stale AUs with fresh dequeue times; repeated texture
draw expiration and epoch races; native Opus stale-media rejection and resumption.
Existing burst/loss/backpressure, 30-minute RTP/audio and 20-minute renderer,
Home refresh/reconnect, voice, diagnostics, and lifecycle tests remain required.

Host fakes validate software ownership/admission and protocol state, not Vita
AVCDEC throughput, GPU scanout, DAC latency, or Xbox's counterfactual response.
No hardware overhead measurement or sustained gameplay acceptance is claimed.
The clock state is constant-size; packet checks are O(1) with short mutex sections
and no I/O. Ordinary P AUs keep one existing H.264 inspection pass. New raw cache
storage is capped; parameter parsing adds bounded bookkeeping and transient
copying only for parameter updates/recovery. No lock spans a native decode call.

## Verification provenance

The final GitHub workflow is authoritative for the final source SHA: host Rust
1.98.1, all host harnesses/strict Clippy, vendored SCTP tests, and native target
test compilation/strict Clippy/release ELF build using the unchanged pinned SDK
`ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc`.
ELF verification is not VPK verification; this change does not package a VPK or
declare a release candidate. Final run identity/results are recorded in the
engineering response after the gates finish.
