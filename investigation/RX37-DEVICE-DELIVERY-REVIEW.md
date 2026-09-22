# RX37.4 device result: shared delivery stalls, likely Home

Jordan supplied pipeline-history.txt and pipeline-incidents.csv after testing
RX37.4 and clarified, "I think this was home." Treat the mode as likely Home,
not confirmed Cloud. The history identifies the build and revision but does not
record the session mode. Server-reported 3840x2160 is not a reliable mode label;
the actual H.264 SPS remains 1280x720, Main, refs1, reorder0.

## Evidence identity and limits

- Runtime: 70ccd93aee1c99bb8550344c264b0b6413ece1d9, RX Test37.4.
- History SHA256: fe07817501a536e3e34c4d5d729fb426c58c73690e97c734ce9bcf2c02eb2be2.
- Incidents SHA256: e636cda77793c836ca16cf780298eb97ea0d1fa8e44308160ca17d15571e9e83.
- History: 921 snapshots, 1.277-931.567s; roughly 15.5 minutes.
- Incidents: 1,024 retained events, 664.689-932.313s. This is a bounded tail,
  not a complete-session incident count.
- Relative RTP arrival growth is delay added against the fastest observed
  arrival, not calibrated end-to-end latency. The absolute SR offset around
  16.7 seconds is clock bias and is not evidence of 16.7 seconds of latency.
- One-second snapshot medians are not individual-frame percentiles. An empty
  decoder queue does not prove the network path or socket buffers are empty.

## Findings

| Observation | Evidence | Implication |
|---|---|---|
| Ordinary decoding keeps pace | Median decoded60/s, shown59/s; queue age0ms; median measured decoder residence57ms and receive-to-GPU66ms | RX37 does not exhibit a continuously growing local decode queue |
| Shared delivery slows sharply | At14.297s video payload132kbps; at19.300s178kbps, versus1842kbps at12.294s | Stalls precede the later catch-up queue pressure |
| Audio and video arrive progressively later together | At19.300s relative video growth5377ms, audio5264ms; queued AUs0 | Cannot attribute this to the video renderer or compressed-frame queue alone |
| Application receive service remains active | At19.300s426 receive passes, zero budget hits, max1 datagram/pass, RTCpump max2ms | The configured32-packet/2ms receive-pass limit is not being hit in that window |
| Catch-up follows | At26.307s video3117kbps,205 receive-budget hits and three accumulated queue-full drops | Queue exhaustion is downstream amplification of a delivery burst |
| Loss/recovery remains serious | Final counters:5546 reorder-released missing packets,331 sequence-damaged AUs,133 FU-damaged AUs,25 queue-full drops,2521 IDR-wait drops | RX37 is not accepted for this run; counts do not locate where packets were lost |
| Long freezes remain | Retained fresh-picture gap9500.951ms ending669.482531s; final active recovery13570ms | Passing host tests did not establish device playability |

Eleven stretches have both audio and video relative arrival growth >=1s in
consecutive snapshots:15.298-25.305,44.331-53.343,58.348-68.357,
190.528-192.529,413.867-416.870,570.086-576.091,660.203-669.216,
687.236-689.239,881.506-887.513,911.543-914.550,920.555-931.567s.
These are thresholded sampled intervals, not exact incident boundaries. Their
uneven spacing does not establish a fixed periodic timer as the cause.

The retained incident tail contains41 recovery starts,41 recovery ends,
11 hard frame-limit events,206 retained queue-pressure events,68 sequence
drop reasons and21 FU drop reasons. The equal recovery-start/end count does
not imply the last recovery completed: the ring starts during an earlier wait.

The client lowers negotiated REMB demand to500kbps during severe delay and
records successful feedback sends. That is neither acknowledgement that Xbox
applied the request nor evidence that500kbps can traverse the impaired path.
Further reducing bitrate or increasing the32-AU cap is not an established fix.

## Follow-up: extender and pause correlation

Jordan reports that the Xbox is connected by Ethernet to a Wi-Fi extender,
with throughput sometimes reaching approximately 500 Mbps. The extender's uplink
and the Vita's access point are not yet confirmed. Do not describe this as an
entirely wired path, or assume that the extender necessarily has a wireless
uplink. Peak throughput does not locate the observed delivery interruptions.

Jordan also reports that this stretch of issues occurred after putting down
the device while paused, then picking it back up. The exact pause mechanism,
duration, and elapsed time in the recording are not known. This is a useful
correlation, not proof that all 11 stalls had the same cause.

Source review at da620246f9e6a58b765e4867ced9ff7273e897b5:

- `StreamingSession::set_paused` changes the overlay flag and hint timestamp.
  It does not pause or queue RTC media.
- The shell still calls `app.tick`, drains audio, and calls
  `VitaSurface::sync_video_frame` while that overlay is open. Video presentation
  continues at the menu cadence; the RTC worker is a separate thread and has no
  pause command.
- There is no explicit application background/foreground or suspend/resume
  handling in the shell. An in-game pause, GreenVita's overlay, and actual Vita
  sleep must therefore be distinguished before selecting a resume change.
- Upstream SDL2 defaults to disabling the screensaver, and its Vita event pump
  ticks the idle timers when that flag is set. GreenVita regularly pumps SDL
  events, including while paused. This source check does not establish the
  exact linked SDL version or actual device power state, and does not justify
  presenting another idle-timer call as a demonstrated fix.

SDL references reviewed:
[video initialization](https://github.com/libsdl-org/SDL/blob/SDL2/src/video/SDL_video.c)
and [Vita event pump](https://github.com/libsdl-org/SDL/blob/SDL2/src/video/vita/SDL_vitavideo.c).

## Next decision

The logs narrow the fault to media delivery upstream of AU admission, which
still includes Vita networking/driver behavior, Wi-Fi/router conditions, Xbox
sending, and a potentially remote route. They do not isolate one of these.
Follow-up: Jordan clarified that only the game was paused; GreenVita's menus
were not open. He also reports Cloud delay and supplied a second capture.
See [the second device review](RX37-SECOND-DEVICE-REVIEW.md) for the early-start
delivery slowdown and a controlled Wi-Fi power-saving comparison on RX37.4.
Keep the RX35.2 Home rollback available and do not mix Home and Cloud results.

No new runtime adjustment or VPK is issued from this evidence alone. Keep the
RX37 implementation and failed device evidence distinct from a proven remedy.
Future runtime work should record Home/Cloud and the selected ICE route in
history so mode and local-versus-remote routing can be established directly;
that diagnostic improvement alone must not be presented as a lag fix.
