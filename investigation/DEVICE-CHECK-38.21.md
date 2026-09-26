# RX38.21: one consolidated device check

Status: code corrections and host verification are complete; actual Vita/Xbox
latency and sustained-play acceptance are pending. This is not a claim that the
reported incident is definitively solved.

## Install and run

Install `GreenVita-RX-Test-38.21.vpk` over the existing RX Test application
(`GRNVTEST1`). Do not uninstall or delete account/configuration data. Confirm
the build displays `38.21` / revision `49baec0` before starting.

Keep the same network and stream settings. Run one Home session for 30 minutes,
including an in-game pause/resume and one manual stream refresh. Check action
response at startup and roughly 2, 5, 15 and 30 minutes. Toggle microphone and
diagnostics during the session; check voice, mute and the retained controls.
Refresh must leave the same game running. Then run Cloud for 30 minutes on the
same candidate. Stop early if either is clearly unplayable; there is no value in
repeated restarts to obtain a favorable sample.

## What this resolves

- A pass supports sustained play on this particular device/network, including
  the previously problematic first minutes and pause/resume.
- A failure with a reduced advertised bitrate but an unchanged received rate
  challenges the assumption that the Xbox obeys receiver constraints.
- A failure after arrival timing and received rate settle points to a separate
  local decoder, presentation or input problem. Long old-picture tails after
  paused/static video remain an explicit possibility.

The acceptance limits are unchanged: early (minutes 2–5) versus late (25–30)
p95 latency growth at most 50 ms; no added media delay over 250 ms lasting 5 s;
no measured action-to-visible response over 500 ms or recurrence of 5–6 s lag;
A/V relationship returns within 100 ms of its baseline; isolated recovery within
1 s without recurring request storms; Home refresh preserves the running game.
App receive-to-GPU measurements do not measure physical input-to-display latency.
Visual observation can flag a failure, but precise physical latency needs a
recording with visible input and screen response.

## Evidence already collected by the app

Exit each stream normally, not by force-closing the application. It writes:

`ux0:data/green-vita-540-test/pipeline-status.txt`
`ux0:data/green-vita-540-test/pipeline-history.txt`
`ux0:data/green-vita-540-test/pipeline-incidents.csv`
`ux0:data/green-vita-540-test/pipeline-trace.csv`

One previous stream is retained as `pipeline-previous-*`. After Home followed
by Cloud, preserve both sets before starting another session. Mode and revision
are now explicit. `receiver_ceiling_bps` and `receiver_ceiling_delay_ms` record
target changes so the analysis can compare feedback with actual delivered rate
and delay. History retains about 35 minutes; the detailed trace is only a short
rolling tail, not a full packet capture.

No additional build variants or settings sweep are required for this check.

## Rollback

Reinstall a retained prior VPK under the same title ID, without deleting data.
The prior RX38.20 package is identified in `CANDIDATE-38.20-BUILD.json` but is
known to have the reported latency failure, not a known-good baseline. RX38.10
is only a user-observed good-startup comparison; it lacks later Home-refresh
and lifecycle safeguards, so a wholesale rollback is not the proposed fix.

