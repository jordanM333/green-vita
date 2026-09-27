#!/usr/bin/env python3
"""Optional second observation: correlate a short header-only tcpdump PCAP with the Vita bundle.

No packet payloads are exported. Only classic PCAP with Ethernet, raw IPv4, or
Linux cooked headers is supported. PCAPNG is rejected, never misinterpreted.
"""
import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import struct
from analyze_diagnostic import UNKNOWN, events_from, packet_key, read_bundle, stats


def pcap_packets(data):
    formats = {b"\xd4\xc3\xb2\xa1": ("<", 1), b"\xa1\xb2\xc3\xd4": (">", 1),
               b"\x4d\x3c\xb2\xa1": ("<", .001), b"\xa1\xb2\x3c\x4d": (">", .001)}
    if data[:4] not in formats or len(data) < 24:
        raise ValueError("classic PCAP required; convert PCAPNG with editcap -F pcap")
    endian, scale = formats[data[:4]]
    link = struct.unpack_from(endian+"I", data, 20)[0]
    if link not in (1, 101, 113, 276):
        raise ValueError(f"unsupported PCAP link type {link}")
    offset = 24
    while offset < len(data):
        if len(data)-offset < 16:
            raise ValueError("truncated PCAP record header")
        seconds, sub, size, _ = struct.unpack_from(endian+"IIII", data, offset)
        offset += 16
        if size > len(data)-offset:
            raise ValueError("truncated PCAP record")
        b = data[offset:offset+size]
        offset += size
        ip = {1:14, 101:0, 113:16, 276:20}[link]
        if link == 1:
            if len(b) < 14:
                continue
            typ = struct.unpack_from("!H", b, 12)[0]
            while typ in (0x8100, 0x88a8) and len(b) >= ip+4:
                typ = struct.unpack_from("!H", b, ip+2)[0]
                ip += 4
            if typ != 0x800:
                continue
        if len(b) < ip+20 or b[ip] >> 4 != 4 or b[ip+9] != 17:
            continue
        if struct.unpack_from("!H", b, ip+6)[0] & 0x3fff:
            continue  # fragmented packets are not reconstructed or guessed
        ihl = (b[ip] & 15)*4
        rtp = ip+ihl+8
        if ihl < 20 or len(b) < rtp+12 or b[rtp] >> 6 != 2 or 192 <= b[rtp+1] <= 223:
            continue
        total_length = struct.unpack_from("!H", b, ip+2)[0]
        udp_length = struct.unpack_from("!H", b, ip+ihl+4)[0]
        header_length = 12 + 4*(b[rtp] & 15)
        if total_length < ihl+8+header_length or not 8+header_length <= udp_length <= total_length-ihl:
            continue
        seq, timestamp, ssrc = struct.unpack_from("!HII", b, rtp+2)
        yield (ssrc, seq, timestamp), seconds*1_000_000 + sub*scale


def correlate(bundle, pcap, point, capture_drops=None):
    data = read_bundle(bundle)
    manifest = json.loads(data["manifest.json"])
    if manifest.get("schema") != 1:
        raise ValueError("unsupported capture schema")
    events = events_from(data["events.csv"])
    observed = defaultdict(list)
    for key, at in pcap_packets(Path(pcap).read_bytes()):
        observed[key].append(at)
    # The external header has no session epoch or track generation. Require a
    # unique identity on BOTH sides; never assign one observed packet to two
    # device streams after a reset, or silently choose a retransmission.
    identity = lambda e: (e["ssrc"], e["seq"], e["rtp"])
    packets = [e for e in events if e["stage"] == "rtc" and e["media"] in (1, 2) and (e["flags"] & 5) == 5]
    device_counts = Counter(identity(e) for e in packets)
    raw = Counter(packet_key(e, e["at_us"]) for e in events if e["stage"] == "udp" and e["flags"] & 1)
    groups, generations = defaultdict(list), defaultdict(int)
    missing = ambiguous = invalid_dequeue = unmatched_udp = 0
    for e in events:
        if e["stage"] == "track_open":
            generations[e["epoch"], e["media"]] += 1
        if e["stage"] != "rtc" or e["media"] not in (1, 2) or not e["flags"] & 4:
            continue
        if not e["flags"] & 1 or e["a"] == UNKNOWN or e["a"] > e["at_us"]:
            invalid_dequeue += 1
            continue
        candidates = observed[identity(e)]
        if len(candidates) != 1 or device_counts[identity(e)] != 1:
            missing += not candidates
            ambiguous += len(candidates) > 1 or device_counts[identity(e)] != 1
            continue
        if raw[packet_key(e, e["a"])] != 1:
            unmatched_udp += 1
            continue
        groups[e["epoch"], e["media"], generations[e["epoch"], e["media"]], e["ssrc"]].append((e, candidates[0]))
    results = []
    for identity, packets in groups.items():
        first, first_observer = packets[0]
        rate = first["c"]
        if not 0 < rate < UNKNOWN:
            continue
        deltas, windows = [], defaultdict(list)
        last_rtp, ticks = first["rtp"], 0
        backwards = discontinuities = 0
        previous = first
        for e, at in packets:
            forward = (e["rtp"]-last_rtp) & 0xffffffff
            if forward >= 1 << 31:
                backwards += 1
                if ((last_rtp-e["rtp"]) & 0xffffffff) > 2*rate:
                    discontinuities += 1
                continue
            if forward*1_000_000/rate > max(0, e["a"]-previous["a"])+500_000 or e["c"] != rate:
                discontinuities += 1
            ticks += forward
            last_rtp, previous = e["rtp"], e
            travel_change = (e["a"]-first["a"])-(at-first_observer)
            deltas.append(travel_change)
            windows[e["a"]//1_000_000].append((travel_change,
                at-first_observer-ticks*1_000_000/rate,
                e["a"]-first["a"]-ticks*1_000_000/rate))
        timeline = [{"second": s, "matched_packets": len(values),
                     "observer_to_dequeue_change_plus_clock_skew": stats([v[0] for v in values]),
                     "observer_vs_nominal_media_change": stats([v[1] for v in values]) if not discontinuities else None,
                     "dequeue_vs_nominal_media_change": stats([v[2] for v in values]) if not discontinuities else None}
                    for s, values in sorted(windows.items())]
        results.append({"stream": identity, "matched": len(packets),
                        "baseline": {"ssrc": first["ssrc"], "seq": first["seq"], "rtp": first["rtp"],
                                     "dequeue_us": first["a"], "observer_us": first_observer},
                        "observer_to_dequeue_change_plus_clock_skew": stats(deltas),
                        "timestamp_discontinuities": discontinuities, "reordered_timestamps": backwards,
                        "dequeue_windows": timeline,
                        "observer_wall_progress_us": packets[-1][1]-first_observer,
                        "dequeue_wall_progress_us": packets[-1][0]["a"]-first["a"]})
    return {"observation_point": point, "streams": results, "missing_packets": missing,
            "ambiguous_packet_identities": ambiguous,
            "invalid_dequeue_observations": invalid_dequeue, "unmatched_udp_observations": unmatched_udp,
            "capture_drops": capture_drops, "device_lock_skips": manifest.get("lock_skips"),
            "outcome": "CORRELATED_OBSERVATIONS_ONLY" if any(
                r["stream"][1] == 1 and r["matched"] >= 10 and r["dequeue_wall_progress_us"] >= 500_000
                and not r["timestamp_discontinuities"] for r in results) else "INCONCLUSIVE",
            "limitations": "Absolute clock offset cancels; clock skew/steps do not. Compare video with audio and check capture drops/clock health before attributing differential growth. The chosen point only bounds the interval it actually observes; this cannot name Xbox or Wi-Fi as a cause by itself."}


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("bundle", type=Path)
    p.add_argument("pcap", type=Path)
    p.add_argument("--point", required=True, help="physical capture interface and position, e.g. AP wired ingress")
    p.add_argument("--capture-drops", type=int, help="tcpdump's reported kernel drop count; omit if unknown")
    a = p.parse_args()
    if a.capture_drops is not None and a.capture_drops < 0:
        p.error("capture drops cannot be negative")
    print(json.dumps(correlate(a.bundle, a.pcap, a.point, a.capture_drops), indent=2))
