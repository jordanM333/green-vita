#!/usr/bin/env python3
"""Check release identity, archive integrity, isolation and synthetic media format."""
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import zipfile
from verify_livearea import check_archive

path = Path(sys.argv[1])
with zipfile.ZipFile(path) as archive:
    assert archive.testzip() is None, "Corrupt VPK entry"
    livearea = check_archive(archive)
    required = {"eboot.bin", "sce_sys/param.sfo", "sce_sys/icon0.png", "control.mp4", "build-info.json", "README.md"}
    assert required <= set(archive.namelist()), "Missing required files"
    assert archive.read("eboot.bin")[:4] == b"SCE\0", "Missing Vita SELF header"
    sfo = archive.read("sce_sys/param.sfo")
    magic, _, keys, values, count = struct.unpack_from("<5I", sfo)
    assert magic == 0x46535000, "Invalid SFO"
    fields = {}
    for i in range(count):
        key, fmt, length, _, offset = struct.unpack_from("<HHIII", sfo, 20 + i * 16)
        name = sfo[keys + key:].split(b"\0", 1)[0].decode()
        value = sfo[values + offset:values + offset + length]
        fields[name] = value.rstrip(b"\0").decode() if fmt == 0x204 else value.hex()
    assert fields["TITLE_ID"] == "GVTVPRB01", fields
    assert fields["TITLE"] == "Vita TV Probe", fields
    assert fields["APP_VER"] == "00.03", fields
    info = json.loads(archive.read("build-info.json"))
    assert info["device_tested"] is None and info["firmware_tested"] is None
    assert not any(info["protected_playback_verified"].values())
    assert info["title_id"] == fields["TITLE_ID"]
    assert info["version"] == "0.3"
    extracted = path.with_suffix(".control.mp4")
    extracted.write_bytes(archive.read("control.mp4"))
    try:
        media = json.loads(subprocess.check_output([
            "ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", str(extracted)
        ]))
        video = next(s for s in media["streams"] if s["codec_type"] == "video")
        audio = next(s for s in media["streams"] if s["codec_type"] == "audio")
        assert (video["codec_name"], video["width"], video["height"]) == ("h264", 480, 272)
        assert "Baseline" in video["profile"] and video["level"] == 30
        assert (audio["codec_name"], audio["channels"], audio["sample_rate"]) == ("aac", 2, "48000")
        assert abs(float(media["format"]["duration"]) - 12) < 0.1
    finally:
        extracted.unlink(missing_ok=True)
    print(json.dumps({"package": str(path), "bytes": path.stat().st_size,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "title_id": fields["TITLE_ID"],
        "source_commit": info["source_commit"], "package_checks": "passed", "livearea_images": livearea,
        "vita_runtime_test": "NOT_PERFORMED", "protected_playback": "NOT_VERIFIED"}, indent=2))
