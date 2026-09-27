#!/usr/bin/env python3
"""Fail before compilation when the specialized Vita SDK is unavailable or mismatched."""
import os
from pathlib import Path
import subprocess
import sys

EXPECTED_RUST = "rustc 1.97.0-nightly (4b0c9d76a 2026-05-10)"
ROOT = Path(__file__).resolve().parents[1]
IMAGE = (ROOT / "tools/sdk-image.txt").read_text().strip()

def main():
    sdk = os.environ.get("VITASDK")
    if not sdk:
        sys.exit(f"VITASDK is missing. Run inside the pinned SDK container: {IMAGE}. See README.md.")
    for path in ["bin/arm-vita-eabi-gcc", "bin/vita-make-fself", "arm-vita-eabi/include/psp2/videodec.h"]:
        if not (Path(sdk) / path).is_file():
            sys.exit(f"Incomplete Vita SDK: missing {path}. Use the pinned container in README.md.")
    try:
        rust = subprocess.check_output(["rustc", "--version"], text=True).strip()
        vita = subprocess.run(["cargo", "vita", "--version"], capture_output=True, text=True)
    except OSError:
        sys.exit("Rust/cargo-vita unavailable. Use the pinned SDK container in README.md.")
    if rust != EXPECTED_RUST:
        sys.exit(f"Rust toolchain mismatch: expected {EXPECTED_RUST}; found {rust}.")
    if vita.returncode:
        sys.exit("cargo-vita unavailable. Use the pinned SDK container in README.md.")
    supplied_image = os.environ.get("GREENVITA_SDK_IMAGE")
    if supplied_image and supplied_image != IMAGE:
        sys.exit("SDK image identity differs from tools/sdk-image.txt.")
    print(f"SDK configuration: {rust}; {vita.stdout.strip()}; expected image {IMAGE}")
    print("Tool versions checked; local environment is not cryptographic attestation of the container.")

if __name__ == "__main__":
    main()
