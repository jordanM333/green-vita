# RX38.15 returned recording: nonblocking keyframe catch-up

RX38.15 has **not resolved latency**. The returned status identifies source
165f42963e1fe6d1754a6e836fd3ba06c1bc7e5a. This review uses the four supplied
history/status/incident/trace files. Raw recordings remain outside the repository.

## Findings

* Arrival feedback is active: two TWCC-bound tracks, 41,181 observed transport
  sequence extensions and 2,858 transmitted TWCC datagrams. Lack of negotiated
  feedback no longer explains this recording's delay.
* Added video arrival delay reaches 1,991 ms at 290.229 s while ICE round trip
  is 2 ms. At that sample, average local receive-to-GPU time is 190 ms. Other
  sustained episodes also place most of the added delay before local decode.
  These are relative RTP arrival measurements, not synchronized capture latency.
* Video reordering finishes with 625 filled holes and zero released losses.
  There are no sequence/FU/malformed-AU drops or hardware decode errors. This
  does not establish which upstream component holds the video, nor rule out
  network delay; it does rule out unrepaired video packet loss as the cause of
  these particular recovery transitions.
* The adaptive REMB request falls to 500 kbps, but incoming video repeatedly
  exceeds that request. A transmitted limit is not an acknowledgment from Xbox.
  Simply reducing it further is not a demonstrated fix.
* Catch-up bursts fill the bounded 32-AU queue twice. They trigger damage
  recovery at 252.496 s and 290.566 s, then wait another 163 ms and 148 ms for
  an IDR. At the second event, video had already accumulated about two seconds
  of arrival delay. Dropping queued dependent frames then creates another wait.
* The earlier 8,343 ms recovery spans a period without incoming video, while
  audio continues. Its initiating event is outside the retained incident tail;
  it cannot be conclusively attributed to manual Refresh from these files.
* The final sample is healthy again: 62 decoded / 55 shown fps, 76/117 ms local
  receive-to-GPU average/maximum, empty compressed queue and zero arrival growth.

The recording does not support blaming the extender, a periodic UI repaint,
or a proven TWCC defect. It does support replacing destructive refresh and
waiting-for-overflow behavior with an earlier, nonblocking recovery attempt.

## Candidate behavior

1. On advancing video samples, 250 ms added arrival delay or at least eight
   queued AUs must persist for 500 ms before requesting a fresh keyframe.
   Automatic requests are at least five seconds apart. Duplicate/late RTP
   timestamps, stale samples, gaps over 250 ms, short bursts and ongoing damage
   recovery do not accumulate pressure. A request does not reset the measured
   delay baseline or claim that the sender discarded its backlog.
2. Playback continues using its valid reference chain while waiting. Manual
   Home Refresh uses this same pending request; it does not clear packet order,
   abandon a partial AU, stop the session, reconnect, or terminate the game.
3. A complete, capacity-checked IDR containing its own parsed SPS and a complete
   PPS is a safe point to discard queued older AUs. Such an IDR also cuts over
   without a prior request if eight or more AUs are queued. The single RTP
   producer drains old queue entries and releases their byte credits, advances
   the decode epoch, then submits the replacement. One old in-flight hardware
   call may finish; existing epoch checks prevent its pixels being published.
   Firmware is not flushed or recreated. IDRs lacking these parameter sets
   continue through normal decoding; arbitrary dependent pictures are never
   used to replace the queue.
4. Actual missing/corrupt fragments still enter the existing damage recovery.
   Frame/byte bounds remain 32 AUs / 4 MiB. An impossible replacement is rejected
   before invalidating playback. Shutdown drains the retained queue receiver and
   admission explicitly checks worker liveness.
5. Status records request count, pending/completed refreshes; incident tracing
   records `lag_keyframe_request` (added delay ms) and `keyframe_cutover_units`
   (queued AUs discarded). A completed cutover is not proof of low latency.

Microphone negotiation/capture/mute, audio levels, the three translucent bottom
buttons and account catalog behavior are retained.

## Verification and acceptance

The focused RTP suite passes 20 tests. It covers partial-AU preservation during
refresh, continuing P pictures, requiring the keyframe's own parameter sets,
PPS at the end of an Annex-B buffer, natural-keyframe backlog cutover, and actual
packet damage during a pending refresh. Synthesized libx264 720p parameter sets
are used; slice decoding is deliberately outside the fake submission sink.

The decoder suite runs the production worker with a controlled SDK boundary,
not a Vita emulator. Tests cover a full frame/byte queue and an in-flight old
call, replacement admission, following-picture ordering, old-pixel exclusion,
byte-credit reuse, no decoder recreation, and shutdown rejection. Deterministic
policy tests cover request cadence, RTP wrap, stale/paused/late samples, short
bursts and existing damage recovery.

RX38.16 source is 297174d05b0f87c46ea329cd4a99343e7f89c6b0, with the exact tree
of the user-approved local commit bb2ea04. All 164 host test executions and the
native release build passed in workflow 35955793475. Artifact and VPK checksums,
both ZIP CRC checks, embedded revision/build and GRNVTEST1 identity were verified.
Native metadata headroom is 90,972 bytes against the 65,536-byte minimum.
`CANDIDATE-38.16-BUILD.json` records the package identity and complete result.
Device acceptance remains pending.
The change directly removes an avoidable local recovery stall, but a new
keyframe may itself remain in an upstream queue. It must not be called a proven
fix for the one-to-two-second arrival delay.

Play Home past the prior failure window (ideally ten minutes), including an
in-game pause and resume with GreenVita's menu closed. Compare visible response,
arrival delay, request/cutover counters, queue pressure and recovery waits. Test
manual Refresh while a game is running and verify that it stays in that game.
Cloud needs its own result. If cutovers occur but arrival delay stays high, do
not increase request frequency blindly: the next investigation must address
sender pacing/adaptation and delivery before the local decoder.
