#!/usr/bin/env python3
"""Offline evidence analysis; never infers Xbox latency from client queues.

No third-party dependencies. Trace durations use the recorder's local clock.
Camera CSV uses presentation times from ONE recording per trial, in seconds:
run,mode,phase,trial,press_s,tv_s,vita_s,uncertainty_ms
tv_s is optional. uncertainty_ms is the bound on EACH annotated event time,
including frame duration/occlusion, not the nominal recording frame rate alone.
"""
import argparse
import csv
import json
import math
import re
import statistics
from collections import Counter, defaultdict
from pathlib import Path


def describe(values):
    if not values:
        return None
    values = sorted(values)
    return {"n": len(values), "min": values[0], "median": statistics.median(values),
            "p95": values[math.ceil(.95 * len(values)) - 1], "max": values[-1]}


def trace_summary(rows):
    times = [int(r["elapsed_us"]) for r in rows]
    if any(b < a for a, b in zip(times, times[1:])):
        raise ValueError("Non-monotonic trace: split sessions before analysis")
    # These stage VALUES are durations. Generation IDs and raw PTS are not.
    stages = {
        "decode_submit": "receive_to_decode_submit_ms",
        "decode_return": "decode_call_ms",
        "decoder_residence_us": "matched_decoder_residence_ms",
        "receive_to_gpu_done_us": "matched_receive_to_gpu_ms",
        "decoded_to_gpu_done_us": "decoded_to_gpu_ms",
        "gpu_queue_wait_us": "gpu_callback_wait_ms",
    }
    measurements = defaultdict(list)
    counts = Counter()
    for row in rows:
        stage = row["stage"]
        counts[stage] += 1
        if stage in stages:
            value = int(row["value_us_or_reason"])
            if value < 0:
                raise ValueError("Negative trace duration")
            measurements[stages[stage]].append(value / 1000)
    return {
        "rows": len(rows),
        "retained_start_s": times[0] / 1e6 if times else None,
        "retained_end_s": times[-1] / 1e6 if times else None,
        "retained_span_s": (times[-1] - times[0]) / 1e6 if times else 0,
        "event_counts": dict(counts),
        "local_measurements": {name: describe(measurements[name]) for name in stages.values()},
        "decoder_output_sequence": decoder_output_sequence(rows),
        "end_to_end_latency_ms": None,
        "limits": ["Retained span is not session duration.",
                   "decode_submit includes assembly and output-surface wait; it is not queue-only age.",
                   "decode_return is call duration, not matched output residence.",
                   "Legacy picture_produced uses submission identity; excluded from output age.",
                   "GPU callback completion is not physical panel scanout.",
                   "No input delivery, network one-way delay, or AV sync inference."],
    }


def decoder_output_sequence(rows):
    """Audit returned PTS, including outputs subsequently rejected by epoch checks.

    A record without an output is NOT a hardware queue-depth measurement: it can
    be parameter-only, rejected, discarded, or outside this ring's retained span.
    Raw output PTS is extended 90 kHz RTP; compare its low word with submissions.
    Never associate a picture using the submission column on decoder_output_pts.
    """
    submitted = defaultdict(list)
    outputs = []
    seen_pts = set()
    for row in rows:
        stage = row["stage"]
        if stage not in ("decode_submit", "decoder_output_pts"):
            continue
        if "submission_rtp_timestamp" not in row:
            continue  # Legacy records without identity cannot establish residence.
        now = int(row["elapsed_us"])
        current = int(row["submission_rtp_timestamp"])
        if stage == "decode_submit":
            submitted[current].append((now, int(row["value_us_or_reason"])))
            continue
        pts = int(row["value_us_or_reason"])
        known = pts != (1 << 64) - 1
        returned = pts & 0xffffffff if known else None
        candidates = submitted.get(returned, []) if known else []
        unique = len(candidates) == 1
        distance = (current - returned) & 0xffffffff if known else None
        current_candidates = submitted.get(current, [])
        outputs.append({
            "elapsed_us": now,
            "current_submission_rtp": current,
            "returned_pts": pts,
            "returned_rtp": returned,
            "duplicate_returned_pts": known and pts in seen_pts,
            "association": "retained_unique_submission" if unique else
                           "ambiguous_submission" if candidates else
                           "unknown_pts" if not known else "submission_not_retained",
            "submit_to_output_ms": (now - candidates[0][0]) / 1000 if unique else None,
            "current_submission_receive_age_ms": current_candidates[-1][1] / 1000
                if current_candidates else None,
            "rtp_lead_ms": distance / 90 if distance is not None and distance < (1 << 31) else None,
        })
        if known:
            seen_pts.add(pts)
    return {
        "outputs": outputs,
        "submit_to_output_ms": describe([o["submit_to_output_ms"] for o in outputs
                                         if o["submit_to_output_ms"] is not None]),
        "limits": ["Trace-event residence has small instrumentation skew relative to the HUD's Instants.",
                   "RTP lead is source-timeline separation, not network latency or a frame count.",
                   "Missing submissions at the start of the ring are not unmatched hardware PTS.",
                   "An outstanding metadata record is not proof of a buffered picture.",
                   "This cannot determine firmware readiness or whether an empty-input call would drain it."],
    }


def history_metrics(body):
    patterns = {
        "rtp_packets": r"RTP pk:(\d+)",
        "decode_calls": r"FPS hwCall:(\d+)",
        "decoded": r"FPS hwCall:\d+ decoded:(\d+)",
        "shown": r"FPS hwCall:\d+ decoded:\d+ shown:(\d+)",
        "queue_depth": r"Q depth/max:(\d+)/",
        "au_age_avg_ms": r"AUage:(\d+)/",
        "decode_call_avg_ms": r"AUage:\d+/\d+ms dec:(\d+)/",
        "decoder_residence_avg_ms": r"Frame age: decoder:(\d+)/",
        "decoder_residence_max_ms": r"Frame age: decoder:\d+/(\d+)ms",
        "receive_to_gpu_avg_ms": r"receiveToGPU:(\d+)/",
        "receive_to_gpu_max_ms": r"receiveToGPU:\d+/(\d+)ms",
        "video_arrival_growth_ms": r"SR offset\* V:[^\n]*?rel\+(\d+)ms",
        "audio_arrival_growth_ms": r"SR offset\* V:[^\n]*? A:[^\n]*?rel\+(\d+)ms",
        "recovery_wait_ms": r"Recovery wait:(\d+)ms",
        "recovery_max_ms": r"Recovery wait:\d+ms max:(\d+)ms",
        "resyncs_process_total": r"resync:(\d+)",
        "decoder_resets_process_total": r"resync:\d+ reset:(\d+)",
        "audio_sdl_ms": r"Delay SDL:(\d+)ms",
        "input_admission_avg_ms": r"Input local:(\d+)/",
        "input_outstanding_sample_bytes": r"Input transport sampled:(\d+)/",
        "input_deferred_process_total": r"Input transport sampled:[^\n]* deferred:(\d+)",
        "remb_target_kbps": r"REMB:[^\n]*?target:(\d+)k",
    }
    result = {}
    for key, pattern in patterns.items():
        match = re.search(pattern, body)
        result[key] = int(match.group(1)) if match else None
    # The recorder resets at connection start; process-wide metric windows do not.
    # Keep those raw samples, but never use them as the new session's baseline.
    result["startup_or_inherited_window"] = (result["rtp_packets"] == 0 or
                                                  "H264: waiting for SPS" in body)
    return result


def history_summary(text):
    blocks = re.split(r"(?m)^elapsed_ms=(\d+)\s*$", text)
    samples = []
    previous = -1
    for i in range(1, len(blocks), 2):
        elapsed = int(blocks[i])
        if elapsed <= previous:
            raise ValueError("Repeated or reversed history time: split sessions")
        previous = elapsed
        body = blocks[i + 1]
        build = re.search(r"Build: RX Test (\S+) revision (\S+)", body)
        samples.append({"elapsed_ms": elapsed,
                        "build": build.group(1) if build else None,
                        "revision": build.group(2) if build else None,
                        "metrics": history_metrics(body),
                        "status": body.strip()})
    valid = [s for s in samples if not s["metrics"]["startup_or_inherited_window"]
             and s["metrics"]["decoder_residence_avg_ms"] is not None]
    change = None
    if len(valid) >= 2 and len({s["revision"] for s in valid}) == 1:
        first, last = valid[0], valid[-1]
        change = {"first_elapsed_ms": first["elapsed_ms"], "last_elapsed_ms": last["elapsed_ms"],
                  "change_in_window_average_ms": last["metrics"]["decoder_residence_avg_ms"] -
                                                 first["metrics"]["decoder_residence_avg_ms"],
                  "meaning": "Change between first/last retained valid local decoder windows; not button latency."}
    return {"samples": samples, "local_decoder_window_change": change,
            "end_to_end_latency_ms": None,
            "limits": ["n/a is no measured output, not a fresh display or zero latency.",
                       "Startup metrics can carry over from the previous stream; excluded from window change.",
                       "Counters marked process_total are not per-session counts.",
                       "Do not sum window averages, RTP growth and recovery maxima into end-to-end delay.",
                       "One-second snapshots cannot establish subsecond causal ordering."],
            "clock_warning": "SR offset and V-A are uncalibrated; do not treat as latency or AV sync."}


def camera_summary(rows):
    trials = []
    seen = set()
    groups = defaultdict(lambda: defaultdict(list))
    for row in rows:
        key = (row["run"], row["mode"], row["phase"], row["trial"])
        if key in seen:
            raise ValueError("Duplicate camera trial")
        seen.add(key)
        if row["mode"] not in ("Home", "Cloud") or row["phase"] not in ("early", "late", "idle_resume", "network_recovery"):
            raise ValueError("Unknown mode/phase; keep recovery separate")
        press, vita, error = (float(row[k]) for k in ("press_s", "vita_s", "uncertainty_ms"))
        if not all(math.isfinite(v) for v in (press, vita, error)) or press < 0 or vita < press or error <= 0:
            raise ValueError("Invalid event order or missing annotation uncertainty")
        def interval(start, end):
            duration = (end - start) * 1000
            return {"estimate_ms": duration, "lower_ms": duration - 2 * error,
                    "upper_ms": duration + 2 * error}
        result = dict(zip(("run", "mode", "phase", "trial"), key))
        result["button_to_vita"] = interval(press, vita)
        if row.get("tv_s", "").strip():
            tv = float(row["tv_s"])
            if not math.isfinite(tv) or tv < press:
                raise ValueError("Invalid TV event time")
            result["button_to_tv"] = interval(press, tv)
            # Signed differential: TV may itself be slower. This is not network latency.
            result["vita_minus_tv"] = interval(tv, vita)
        trials.append(result)
        groups[(row["run"], row["mode"])][row["phase"]].append(result["button_to_vita"])
    comparisons = []
    for (run, mode), phases in groups.items():
        comparison = {"run": run, "mode": mode, "phases": {}, "drift_assessment": "insufficient_trials"}
        for phase, values in phases.items():
            comparison["phases"][phase] = describe([v["estimate_ms"] for v in values])
        early, late = phases.get("early", []), phases.get("late", [])
        if len(early) >= 5 and len(late) >= 5:
            med = lambda vs, field: statistics.median(v[field] for v in vs)
            low = med(late, "lower_ms") - med(early, "upper_ms")
            high = med(late, "upper_ms") - med(early, "lower_ms")
            comparison["median_added_latency_ms"] = med(late, "estimate_ms") - med(early, "estimate_ms")
            comparison["annotation_bound_ms"] = [low, high]
            comparison["drift_assessment"] = ("within_100ms_bound" if high <= 100 else
                                                "exceeds_100ms_bound" if low > 100 else "inconclusive_at_100ms")
        comparisons.append(comparison)
    return {"trials": trials, "comparisons": comparisons, "acceptance": "not_established",
            "limits": ["Camera timestamps must share one timeline within each trial.",
                       "TV response includes input, game and TV display delay; it is not an input ACK.",
                       "Stable but sluggish absolute latency does not pass acceptance.",
                       "Duration, repeat runs, resets, audio sync and gameplay regressions require separate evidence."]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", type=Path)
    parser.add_argument("--history", type=Path)
    parser.add_argument("--camera", type=Path)
    args = parser.parse_args()
    if not any((args.trace, args.history, args.camera)):
        parser.error("provide --trace, --history or --camera")
    report = {"issue_status": "open", "acceptance": "not_established"}
    try:
        if args.trace:
            with args.trace.open(newline="") as f:
                report["trace"] = trace_summary(list(csv.DictReader(f)))
        if args.history:
            report["history"] = history_summary(args.history.read_text())
        if args.camera:
            with args.camera.open(newline="") as f:
                report["camera"] = camera_summary(list(csv.DictReader(f)))
    except (ValueError, KeyError, OSError) as error:
        parser.error(str(error))
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
