#!/usr/bin/env python3
"""Verify the packaged SELF/SFO and record checkout/toolchain provenance.

Does not execute the artifact. Byte identity is not hardware validation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import zipfile


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def sfo_strings(data):
    magic, version, keys, values, count = struct.unpack_from("<4sIIII", data)
    if magic != b"\x00PSF":
        raise ValueError("invalid SFO magic")
    result = {}
    for index in range(count):
        key, fmt, length, maximum, offset = struct.unpack_from("<HHIII", data, 20 + index * 16)
        name = data[keys + key:].split(b"\0", 1)[0].decode()
        value = data[values + offset:values + offset + length]
        if len(value) != length or length > maximum:
            raise ValueError("invalid SFO value bounds")
        if fmt == 0x204:
            result[name] = value.rstrip(b"\0").decode()
    return result


def inspect(vpk, revision, number, diagnostic=False):
    with zipfile.ZipFile(vpk) as archive:
        if len(archive.namelist()) != len(set(archive.namelist())):
            raise ValueError("duplicate package entries")
        bad = archive.testzip()
        if bad:
            raise ValueError(f"package CRC failure: {bad}")
        for name in archive.namelist():
            if name.startswith("/") or ".." in Path(name).parts:
                raise ValueError("unsafe package path")
        executable = archive.read("eboot.bin")
        sfo = sfo_strings(archive.read("sce_sys/param.sfo"))
        for expected in (revision, number):
            if expected.encode() not in executable:
                raise ValueError(f"packaged executable lacks expected identity: {expected}")
        if diagnostic:
            if b"DIAGNOSTIC BUILD" not in executable or b"PROGRESSIVE LATENCY NOT YET FIXED" not in executable:
                raise ValueError("packaged diagnostic warning missing")
            if sfo.get("TITLE") != "GreenVita Diagnostic":
                raise ValueError("diagnostic SFO title missing")
            if executable[:4] != b"SCE\x00":
                raise ValueError("packaged executable is not a Vita SELF")
            if not number.startswith("DIAG03-"):
                raise ValueError("diagnostic build number missing")
            for asset in ("sce_sys/icon0.png", "sce_sys/livearea/contents/template.xml"):
                if not archive.read(asset):
                    raise ValueError(f"empty install asset {asset}")
        if sfo.get("TITLE_ID") != "GRNVTEST1":
            raise ValueError("unexpected install title ID")
    return {"source_commit": revision, "build_number": number,
            "vpk_sha256": sha256(Path(vpk).read_bytes()), "eboot_sha256": sha256(executable),
            "sfo": sfo, "embedded_identity_checked": True, "diagnostic_only": diagnostic,
            "progressive_latency_fixed": False, "zip_crc_checked": True}


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("vpk", type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--number", required=True)
    parser.add_argument("--checkout", action="store_true")
    parser.add_argument("--diagnostic", action="store_true")
    args = parser.parse_args()
    result = inspect(args.vpk, args.revision, args.number, args.diagnostic)
    if args.checkout:
        if git("rev-parse", "HEAD") != args.revision:
            raise SystemExit("workflow checkout does not match embedded revision")
        if git("diff", "--name-only", "HEAD"):
            raise SystemExit("tracked sources changed during build")
        result.update(source_tree=git("rev-parse", "HEAD^{tree}"),
                      cargo_lock_sha256=sha256(Path("Cargo.lock").read_bytes()),
                      rustc=subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
                      workflow_run=os.environ.get("GITHUB_RUN_ID"),
                      workflow_attempt=os.environ.get("GITHUB_RUN_ATTEMPT"),
                      sdk_image=os.environ.get("GREENVITA_SDK_IMAGE"),
                      hardware_validation="not performed")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
