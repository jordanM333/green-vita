#!/usr/bin/env python3
"""Export a copied Vita diagnostic folder as an integrity-checked ZIP."""
import argparse
import hashlib
from pathlib import Path
import zipfile
from analyze_diagnostic import REQUIRED, read_bundle


def export(source, destination):
    data = read_bundle(source)
    destination = Path(destination)
    with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for name in REQUIRED:
            archive.writestr(name, data[name])
    if read_bundle(destination) != data:
        raise ValueError("export byte verification failed")
    return hashlib.sha256(destination.read_bytes()).hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    parser.add_argument("zip", type=Path)
    args = parser.parse_args()
    print(export(args.folder, args.zip))
