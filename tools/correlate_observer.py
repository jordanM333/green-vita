#!/usr/bin/env python3
"""Optional second observation: correlate a short header-only tcpdump PCAP with the Vita bundle.

No packet payloads are exported. Only classic PCAP with Ethernet, raw IPv4, or
Linux cooked headers is supported. PCAPNG is rejected, never misinterpreted.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import struct
from analyze_diagnostic import events_from, read_bundle, stats


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
        seq, timestamp, ssrc = struct.unpack_from("!HII", b, rtp+2)
        yield (ssrc, seq, timestamp), seconds*1_000_000 + sub*scale


def correlate(bundle, pcap, point):
    observed = defaultdict(list)
    for key, at in pcap_packets(Path(pcap).read_bytes()):
        observed[key].append(at)
    groups = defaultdict(list)
    missing = ambiguous = 0
    for e in events_from(read_bundle(bundle)["events.csv"]):
        if e["stage"] != "rtc" or e["media"] not in (1, 2) or not e["flags"] & 4:
            continue
        candidates = observed[e["ssrc"], e["seq"], e["rtp"]]
        if len(candidates) != 1:
            missing += not candidates
            ambiguous += len(candidates) > 1
            continue
        groups[e["epoch"], e["media"], e["ssrc"]].append((e, candidates[0]))
    results = []
    for identity, packets in groups.items():
        first, first_observer = packets[0]
        deltas = [(e["a"]-first["a"])-(p-first_observer) for e,p in packets]
        results.append({"stream": identity, "matched": len(packets),
                        "observer_to_dequeue_change_plus_clock_skew": stats(deltas),
                        "observer_wall_progress_us": packets[-1][1]-first_observer,
                        "dequeue_wall_progress_us": packets[-1][0]["a"]-first["a"]})
    return {"observation_point": point, "streams": results, "missing_packets": missing,
            "ambiguous_packet_identities": ambiguous,
            "outcome": "CORRELATED_OBSERVATIONS_ONLY",
            "limitations": "Absolute clock offset cancels; clock skew/steps do not. Compare video with audio and check capture drops/clock health before attributing differential growth. The chosen point only bounds the interval it actually observes; this cannot name Xbox or Wi-Fi as a cause by itself."}


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("bundle", type=Path)
    p.add_argument("pcap", type=Path)
    p.add_argument("--point", required=True, help="physical capture interface and position, e.g. AP wired ingress")
    a = p.parse_args()
    print(json.dumps(correlate(a.bundle, a.pcap, a.point), indent=2))
