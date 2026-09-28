#!/usr/bin/env python3
"""Derive a timing-only, address-free fixture from the returned DIAG03-8 trace.

Does not reconstruct absent H.264 payloads or fill the recorder's 98s gap.
Keeps every retained nonempty RTP observation in the continuous onset window.
"""
import argparse
import csv
import hashlib
from pathlib import Path

SOURCE_SHA = "7a630fd5f37d656fc5d38940c018025ade2db236003bb0f3459897e8308dfa1b"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("events", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    assert hashlib.sha256(args.events.read_bytes()).hexdigest() == SOURCE_SHA
    with args.events.open() as source:
        rows = [row for row in csv.DictReader(source) if row["stage"] == "rtc"
                and int(row["flags"]) & 4 and 103_000_000 <= int(row["a"]) <= 122_000_000]
    assert len({(row["epoch"], row["ssrc"]) for row in rows if row["media"] == "1"}) == 1
    assert len({(row["epoch"], row["ssrc"]) for row in rows if row["media"] == "2"}) == 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as out:
        out.write("# DIAG03-8 timing-only; original events.csv SHA256=" + SOURCE_SHA + "\n")
        out.write("# D=UDP dequeue, L=RTC delivery, times are original session microseconds.\n")
        out.write("# Missing pre-window history is NOT replayed. No H264 bytes or recovery outcomes.\n")
        out.write("media,D_us,L_us,rtp,sequence\n")
        for row in rows:
            out.write(",".join(row[key] for key in ("media", "a", "at_us", "rtp", "seq")) + "\n")
    print(f"{len(rows)} packet observations; fixture sha256={hashlib.sha256(args.output.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
