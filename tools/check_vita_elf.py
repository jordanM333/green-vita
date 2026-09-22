"""Check the unallocated virtual gap needed by vita-elf-create (ELF32 LE)."""
import argparse
from pathlib import Path
import struct


def metadata_gap(data):
    if len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01":
        raise ValueError("Expected a little-endian ELF32 executable")
    header = struct.unpack_from("<16sHHIIIIIHHHHHH", data)
    if header[2] != 40:
        raise ValueError("Expected ARM ELF")
    entry, phoff, phsize, phcount = header[4], header[5], header[9], header[10]
    if phsize < 32 or not phcount or phoff + phsize * phcount > len(data):
        raise ValueError("Invalid program header table")
    loads = sorted(
        (ph[2], ph[5], ph[6])
        for i in range(phcount)
        if (ph := struct.unpack_from("<IIIIIIII", data, phoff + i * phsize))[0] == 1
    )
    for i, (start, size, flags) in enumerate(loads):
        if flags & 1 and start <= (entry & ~1) < start + size:
            if i + 1 == len(loads):
                raise ValueError("No following load segment to measure")
            return loads[i + 1][0] - ((start + size + 3) & ~3)
    raise ValueError("Entry point is not in an executable load segment")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    args = parser.parse_args()
    gap = metadata_gap(args.elf.read_bytes())
    print(f"SCE metadata virtual headroom: {gap} bytes")
    if gap < 0x10000:
        raise SystemExit("Expected at least 65536 bytes before the next load segment")
