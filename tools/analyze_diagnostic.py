#!/usr/bin/env python3
"""Analyze correlated observations; never promote an upper bound to a cause.

Input is an exported diagnostic folder or ZIP. No third party packages needed.
"""
import argparse
from collections import Counter, defaultdict
import csv
import hashlib
import io
import json
from pathlib import Path
import statistics
import zipfile

UNKNOWN = (1 << 64) - 1
REQUIRED = ("manifest.json", "events.csv", "history.txt", "README.txt")


def read_bundle(path):
    path = Path(path)
    if path.is_dir():
        return {name: (path / name).read_bytes() for name in REQUIRED}
    with zipfile.ZipFile(path) as archive:
        # Allow one containing folder, as created by ordinary desktop ZIP tools.
        result = {}
        for name in REQUIRED:
            matches = [n for n in archive.namelist() if Path(n).name == name]
            if len(matches) != 1:
                raise ValueError(f"expected one {name}, found {len(matches)}")
            item = archive.getinfo(matches[0])
            if item.file_size > 32 * 1024 * 1024:
                raise ValueError("diagnostic member exceeds bounded export size")
            result[name] = archive.read(matches[0])
        return result


def events_from(data):
    rows = []
    seen = set()
    for index, raw in enumerate(csv.DictReader(io.StringIO(data.decode()))):
        e = {k: (v if k in ("section", "stage") else int(v)) for k, v in raw.items()}
        key = tuple((k, v) for k, v in e.items() if k != "section")
        # Cross-section overlap is intentional, but same-section duplicates are evidence.
        if (key, e["section"]) not in seen and any((key, other) in seen for other in ("initial", "onset", "post") if other != e["section"]):
            continue
        seen.add((key, e["section"]))
        e["index"] = index
        rows.append(e)
    return sorted(rows, key=lambda e: (e["at_us"], e["index"]))


def packet_key(e, at):
    return e["epoch"], e["ssrc"], e["seq"], e["rtp"], at


def stats(values):
    values = sorted(values)
    if not values:
        return None
    return {"count": len(values), "min_us": min(values), "median_us": statistics.median(values),
            "p95_us": values[min(len(values)-1, int(len(values)*.95))], "max_us": max(values)}


def analyze(path):
    data = read_bundle(path)
    manifest = json.loads(data["manifest.json"])
    if manifest.get("schema") != 1:
        raise ValueError("unsupported capture schema")
    events = events_from(data["events.csv"])
    raw = defaultdict(list)
    ordered = defaultdict(list)
    generations = defaultdict(int)
    streams = defaultdict(list)
    anomalies = []
    for e in events:
        if e["stage"] == "udp" and e["flags"] & 1:
            raw[packet_key(e, e["at_us"])].append(e)
        elif e["stage"] == "ordered":
            ordered[packet_key(e, e["a"])].append(e)
        elif e["stage"] == "track_open":
            generations[e["epoch"], e["media"]] += 1
        elif e["stage"] == "rtc" and e["media"] in (1, 2) and e["flags"] & 4:
            stream = e["epoch"], e["media"], generations[e["epoch"], e["media"]], e["ssrc"]
            streams[stream].append(e)
    # RTP/NTP pairs constrain sender media-clock consistency, not capture age.
    sender_clocks = defaultdict(list)
    for e in events:
        if e["stage"] == "sender_report":
            sender_clocks[e["epoch"], e["ssrc"]].append(e)
    reports = []
    findings = []
    for stream, packets in streams.items():
        rate = packets[0]["c"]
        if not rate or rate == UNKNOWN:
            anomalies.append("Missing RTP clock rate")
            continue
        matched = unknown = ambiguous = duplicates = reordered = discontinuities = 0
        points = []
        last = None
        last_seq = None
        ticks = 0
        origin = None
        for e in packets:
            key = packet_key(e, e["a"])
            candidates = raw[key]
            bound = None
            if len(candidates) == 1:
                r = candidates[0]
                if r["b"] != UNKNOWN and 0 <= r["b"] <= e["a"]:
                    bound = e["a"] - r["b"]
                matched += 1
            elif len(candidates) > 1:
                ambiguous += 1
            else:
                unknown += 1
            if e["a"] == UNKNOWN or e["a"] > e["at_us"]:
                anomalies.append("Missing/invalid original dequeue timestamp")
                continue
            if last is not None:
                forward = (e["rtp"] - last) & 0xffffffff
                seq_forward = (e["seq"] - last_seq) & 0xffff
                if forward == 0:
                    duplicates += 1  # includes additional fragments of the same frame
                    continue
                if forward >= 1 << 31:
                    reordered += 1
                    # Large backwards timestamps with forward sequence may be an unannounced
                    # source reset. Do not splice clocks or silently call it packet reorder.
                    if (last - e["rtp"]) & 0xffffffff > rate * 2 and 0 < seq_forward < 32768:
                        discontinuities += 1
                    continue
                if forward > rate * 120:
                    discontinuities += 1
                    # Restart an analysis segment, but prohibit attribution across it.
                    ticks = 0
                    origin = None
                else:
                    ticks += forward
            last, last_seq = e["rtp"], e["seq"]
            if origin is None:
                origin = e["a"]
            offset = e["a"] - origin - ticks * 1_000_000 / rate
            released = ordered[key]
            reorder_us = released[0]["at_us"] - e["at_us"] if len(released) == 1 and released[0]["b"] == e["at_us"] else None
            points.append(dict(at=e["at_us"], dequeue=e["a"], offset=offset,
                               rtc=e["at_us"]-e["a"], bound=bound, reorder=reorder_us, ticks=ticks))
        if not points:
            continue
        # One explicit baseline packet is used for media progression at all boundaries.
        # No subtraction of independently minimized relative clocks.
        baseline = min(points, key=lambda p: p["offset"])
        for p in points:
            p["dequeue_growth"] = p["offset"] - baseline["offset"]
            p["rtc_growth"] = p["rtc"] - baseline["rtc"]
            p["application_growth"] = p["dequeue_growth"] + p["rtc_growth"]
            p["pre_socket_lower"] = None if p["bound"] is None else p["dequeue_growth"] - p["bound"]
        delayed = [p for p in points if p["application_growth"] >= 500_000]
        prefix = "video" if stream[1] == 1 else "audio"
        # A run of >=10 observed frames over >=500ms is a sustained positive finding.
        # This does not claim every intervening unobserved packet was equivalent.
        def sustained(samples):
            return len(samples) >= 10 and samples[-1]["at"] - samples[0]["at"] >= 500_000
        pre = [p for p in delayed if p["pre_socket_lower"] is not None and p["pre_socket_lower"] >= 500_000]
        client = [p for p in delayed if p["rtc_growth"] >= 500_000]
        sr_rates = []
        sender_reports = sender_clocks[stream[0], stream[3]]
        for old, new in zip(sender_reports, sender_reports[1:]):
            seconds = (new["a"] - old["a"]) / (1 << 32)
            ticks_sr = (new["b"] - old["b"]) & 0xffffffff
            if 1 <= seconds <= 120 and 0 < ticks_sr < 1 << 31:
                sr_rates.append(ticks_sr / seconds)
        clock_consistent = not any(abs(r / rate - 1) > .02 for r in sr_rates)
        labels = []
        if discontinuities == 0 and clock_consistent:
            if sustained(pre):
                labels.append("DELAY_GROWTH_BEFORE_UDP_SOCKET_BOUND")
            if sustained(client):
                labels.append("MEASURED_CLIENT_RTC_RESIDENCE_GROWTH")
        for label in labels:
            findings.append({"media": prefix, "stream": stream, "finding": label})
        early = [p for p in points if p["at"] <= points[0]["at"] + 2_000_000]
        def interval(samples):
            if not samples:
                return None
            return {"start_us": samples[0]["at"], "end_us": samples[-1]["at"],
                    "dequeue_relative_growth": stats([p["dequeue_growth"] for p in samples]),
                    "rtc_residence": stats([p["rtc"] for p in samples]),
                    "socket_residence_upper_bound": stats([p["bound"] for p in samples if p["bound"] is not None]),
                    "rtc_to_ordered": stats([p["reorder"] for p in samples if p["reorder"] is not None]),
                    "observed_forward_frames": len(samples),
                    "largest_observation_gap_us": max((b["at"]-a["at"] for a,b in zip(samples,samples[1:])), default=0)}
        reports.append({"media": prefix, "stream": stream, "clock_rate": rate,
                        "packet_observations": len(packets), "uniquely_correlated": matched,
                        "missing_udp_observations": unknown, "ambiguous_udp_matches": ambiguous,
                        "same_timestamp_fragments_or_duplicates": duplicates, "reordered_timestamps": reordered,
                        "timestamp_discontinuities": discontinuities,
                        "sender_report_clock_rates": sr_rates,
                        "sender_clock_check": ("inconsistent" if not clock_consistent else "consistent" if sr_rates else "unknown; nominal negotiated RTP rate assumed"),
                        "baseline": interval(early), "delayed": interval(delayed),
                        "deficit_us": points[-1]["offset"] - points[0]["offset"],
                        "media_progress_us": (points[-1]["ticks"] - points[0]["ticks"]) * 1_000_000 / rate,
                        "wall_progress_us": points[-1]["dequeue"] - points[0]["dequeue"],
                        "positive_pre_socket_lower_bounds": stats([p["pre_socket_lower"] for p in pre]),
                        "findings": labels})
    video_findings = [f for f in findings if f["media"] == "video"]
    if video_findings:
        outcome = "BOUNDARY_EVIDENCE_OBTAINED"
    elif manifest.get("trigger_us") is None and any(r["media"] == "video" for r in reports) and not any(r["delayed"] for r in reports if r["media"] == "video"):
        outcome = "NOT REPRODUCED DURING CAPTURE"
    else:
        outcome = "INCONCLUSIVE"
    passes = [e for e in events if e["stage"] == "receive_pass"]
    # Legacy stages identify a frame cohort only when the captured RTC identities
    # are unique across SSRC/track generations. Never attach the latest SSRC blindly.
    cohorts = defaultdict(set)
    for stream, packets in streams.items():
        for p in packets:
            cohorts[stream[0], p["rtp"]].add(stream)
    associated = defaultdict(int)
    unassociated = defaultdict(int)
    for e in events:
        if e["stage"] in ("au_complete", "decode_submit", "picture_output_rtp", "presentation_selected", "audio_decoded", "audio_device_submit"):
            expected = 2 if e["stage"].startswith("audio") else 1
            candidates = [s for s in cohorts[e["epoch"], e["rtp"]] if s[1] == expected]
            (associated if len(candidates) == 1 else unassociated)[e["stage"]] += 1
    missing = ["actual kernel/network arrival timestamps", "capture/encoder time", "physical scanout and audible output"]
    if outcome == "INCONCLUSIVE":
        missing.append("correlated pre-socket packet observation or tighter valid socket bounds")
    return {"outcome": outcome, "progressive_latency_fixed": False, "capture": manifest,
            "streams": reports, "findings": findings, "anomalies": sorted(set(anomalies)),
            "missing_or_unknown": missing,
            "interpretation": "Socket bounds are upper limits only. A large bound cannot establish backlog. Pre-socket growth does not distinguish sender, network or driver delay. RTP progression assumes the negotiated media clock; resets are not bridged.",
            "coverage": {"events": len(events), "lock_skips": manifest.get("lock_skips"),
                         "truncated_records": manifest.get("truncated_records"),
                         "unobserved_intervals": "unknown; absence is never zero residence"},
            "receive_scheduling": {"pass_service": stats([e["at_us"]-e["a"] for e in passes if e["a"] != UNKNOWN]),
                                   "between_passes": stats([e["a"]-e["b"] for e in passes if e["b"] != UNKNOWN and e["a"] >= e["b"]]),
                                   "interpretation": "Between-pass time includes idle wait and other work, not proof of CPU starvation; network arrival rate is unknown."},
            "frame_cohort_association": {"unique": dict(associated), "unknown": dict(unassociated), "scope": "epoch + unique captured track generation/SSRC + RTP; no physical presentation inference"},
            "downstream": {stage: stats([e["a"] for e in events if e["stage"] == stage]) for stage in
                ("au_complete", "h264_assembly_us", "decode_submit", "audio_device_submit", "receive_to_gpu_done_us")},
            "feedback_events": dict(Counter(e["stage"] for e in events if any(w in e["stage"] for w in ("feedback", "keyframe", "ceiling", "recovery", "rtcp_udp", "nack")))),
            "file_sha256": {k: hashlib.sha256(v).hexdigest() for k, v in data.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    print(json.dumps(analyze(args.bundle), indent=2))


if __name__ == "__main__":
    main()
