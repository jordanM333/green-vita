#!/usr/bin/env python3
"""Correlate selected source/upload pixels; keep display and content causes separate."""
import argparse
import csv
import hashlib
import json
from pathlib import Path


def words(text, digits, count):
    if len(text) != count * digits:
        raise ValueError("pixel grid size mismatch")
    return [int(text[i:i+digits], 16) for i in range(0, len(text), digits)]


def analyze(path):
    path = Path(path)
    manifest = json.loads((path / "manifest.json").read_text())
    if manifest["schema"] != 1 or manifest["grid"] != [32, 18]:
        raise ValueError("unsupported pixel capture")
    data = (path / "submitted.h264").read_bytes()
    if len(data) != manifest["encoded_bytes"]:
        raise ValueError("encoded capture size mismatch")
    offset = 0
    aus = []
    for raw in csv.DictReader((path / "access-units.csv").open()):
        a = {k: int(v) for k, v in raw.items()}
        if a["offset"] != offset or a["length"] <= 0 or offset + a["length"] > len(data):
            raise ValueError("invalid access unit extents")
        offset += a["length"]
        aus.append(a)
    if offset != len(data):
        raise ValueError("unindexed encoded bytes")
    observations = []
    identities = set()
    for raw in csv.DictReader((path / "pixels.csv").open()):
        s = {k: int(v) for k, v in raw.items() if k not in ("source_bgr565", "uploaded_bgr565", "display_abgr8888")}
        key = s["decode_epoch"], s["rtp"], s["generation"]
        if key in identities:
            raise ValueError("duplicate selected picture identity")
        identities.add(key)
        source = words(raw["source_bgr565"], 4, 576)
        uploaded = words(raw["uploaded_bgr565"], 4, 576)
        display = words(raw["display_abgr8888"], 8, 576)
        s.update(upload_mismatches=sum(a != b for a, b in zip(source, uploaded)),
                 source_nonblack=sum(v != 0 for v in source),
                 uploaded_nonblack=sum(v != 0 for v in uploaded),
                 framebuffer_nonblack=sum(v & 0xffffff != 0 for v in display) if s["display_result"] >= 0 else None,
                 matching_submitted_aus=sum(a["decode_epoch"] == s["decode_epoch"] and a["rtp"] == s["rtp"] for a in aus))
        observations.append(s)
    mismatch = [s for s in observations if s["upload_mismatches"]]
    # Same selected generation and same grid coordinates permit an exact copy check.
    # Framebuffer API age/overlays and legitimate black source content do not.
    outcome = "SELECTED_PIXEL_COPY_MISMATCH" if mismatch else "INCONCLUSIVE_BLACKOUT_ORIGIN"
    return {"outcome": outcome, "source": manifest["source"], "build": manifest["build"],
            "observations": observations, "access_units": len(aus),
            "encoded_replay_complete": not manifest["encoded_truncated"] and manifest["lock_skips"] == 0,
            "encoded_sha256": hashlib.sha256(data).hexdigest(),
            "physical_scanout_verified": False,
            "limits": "Equal upload samples rule out corruption only at sampled locations. Black decoder-buffer content needs independent H.264 decoding before causal attribution. OS framebuffer may refer to a prior displayed frame and includes UI. Missing pixels remain unknown."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    print(json.dumps(analyze(parser.parse_args().folder), indent=2))
