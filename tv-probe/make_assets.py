#!/usr/bin/env python3
"""Generate original bubble art and an unencrypted control clip; no downloaded media."""
import json
import os
from pathlib import Path
import struct
import subprocess
import zlib
from verify_livearea import check_png

ROOT = Path(__file__).resolve().parent
ASSETS = ROOT / "assets"
ASSETS.mkdir(exist_ok=True)


def png(path, width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

    # LiveArea promotion rejects RGBA icon/background PNGs (0x8010113D).
    # Generate opaque 8-bit indexed PNGs directly, including a full palette.
    palette = [(15, 22, 36), (59, 220, 173), (28, 43, 61), (236, 242, 250)]
    palette += [(0, 0, 0)] * (256 - len(palette))
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            # Television frame and play symbol; no service branding.
            nx, ny = x / width, y / height
            color = 0
            if 0.12 < nx < 0.88 and 0.2 < ny < 0.76:
                color = 1
                if 0.16 < nx < 0.84 and 0.24 < ny < 0.72:
                    color = 2
                    if 0.39 < nx < 0.65 and abs(ny - 0.48) < (0.65 - nx) * 0.7:
                        color = 3
            if 0.39 < nx < 0.61 and 0.79 < ny < 0.83:
                color = 1
            rows.append(color)
    data = b"\x89PNG\r\n\x1a\n"
    data += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 3, 0, 0, 0))
    data += chunk(b"PLTE", bytes(channel for color in palette for channel in color))
    data += chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    check_png(data, (width, height), str(path))
    path.write_bytes(data)


png(ASSETS / "icon0.png", 128, 128)
png(ASSETS / "bg.png", 840, 500)
png(ASSETS / "startup.png", 280, 158)
(ASSETS / "template.xml").write_text('''<?xml version="1.0" encoding="utf-8"?>
<livearea style="a1" format-ver="01.00" content-rev="1">
  <livearea-background><image>bg.png</image></livearea-background>
  <gate><startup-image>startup.png</startup-image></gate>
</livearea>
''')

subprocess.run([
    "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
    "-f", "lavfi", "-i", "testsrc2=size=480x272:rate=30",
    "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
    "-t", "12", "-c:v", "libx264", "-profile:v", "baseline", "-level", "3.0",
    "-pix_fmt", "yuv420p", "-bf", "0", "-refs", "1", "-g", "30", "-crf", "25",
    "-c:a", "aac", "-b:a", "96k", "-ac", "2", "-movflags", "+faststart",
    str(ASSETS / "control.mp4")
], check=True)
(ASSETS / "build-info.json").write_text(json.dumps({
    "app": "Vita TV Probe", "version": "0.3", "title_id": "GVTVPRB01",
    "source_commit": os.environ.get("GITHUB_SHA", "local-unpublished"),
    "workflow_run": os.environ.get("GITHUB_RUN_ID", "local"),
    "sdk_image": "ghcr.io/vita-rust/vitasdk-rs@sha256:351f167c6c0c502baf92502b779cc4b52e9f82ac83efd172911c3ce37b3199cc",
    "device_tested": None, "firmware_tested": None,
    "protected_playback_verified": {"Apple TV": False, "Netflix": False, "Hulu": False, "HBO Max": False},
    "control_clip": "Original synthetic 12s 480x272 30fps H264 Baseline L3.0 / AAC stereo 48kHz 440Hz tone"
}, indent=2) + "\n")
