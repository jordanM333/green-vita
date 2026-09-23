"""Validate the opaque indexed PNG profile used by this project's LiveArea.

Reference: https://github.com/hammerill/livearea-specs
The old RGBA icon/background shipped in 0.1 matched installation error
0x8010113D. An 8-bit-per-channel RGBA image is NOT an 8-bit indexed image.
"""
import struct
import zlib
import xml.etree.ElementTree as ET

IMAGES = {
    "sce_sys/icon0.png": (128, 128),
    "sce_sys/livearea/contents/bg.png": (840, 500),
    "sce_sys/livearea/contents/startup.png": (280, 158),
}


def check_png(data, dimensions, name):
    def require(condition, message):
        if not condition:
            raise ValueError(f"{name}: {message}")

    require(data[:8] == b"\x89PNG\r\n\x1a\n", "invalid PNG signature")
    pos, chunks = 8, []
    while pos < len(data):
        require(pos + 12 <= len(data), "truncated chunk header")
        size = struct.unpack_from(">I", data, pos)[0]
        require(pos + size + 12 <= len(data), "truncated chunk payload")
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + size]
        crc = struct.unpack_from(">I", data, pos + 8 + size)[0]
        require(crc == zlib.crc32(kind + body) & 0xffffffff, "invalid chunk CRC")
        chunks.append((kind, body))
        pos += size + 12
    require(bool(chunks) and chunks[0][0] == b"IHDR", "missing initial IHDR")
    require(len(chunks[0][1]) == 13, "invalid IHDR length")
    width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", chunks[0][1])
    require((width, height) == dimensions, f"incorrect dimensions {width}x{height}")
    require(depth == 8 and color == 3,
            f"requires 8-bit indexed PNG (depth=8, color=3); got depth={depth}, color={color}")
    require((compression, filtering, interlace) == (0, 0, 0), "requires non-interlaced standard PNG")
    kinds = [kind for kind, _ in chunks]
    require(kinds.count(b"IHDR") == 1 and kinds.count(b"PLTE") == 1, "invalid palette/header count")
    require(b"IDAT" in kinds and kinds.index(b"PLTE") < kinds.index(b"IDAT"), "palette must precede image data")
    palette = chunks[kinds.index(b"PLTE")][1]
    require(0 < len(palette) <= 768 and len(palette) % 3 == 0, "invalid palette size")
    require(b"tRNS" not in kinds, "transparency is not used by this project's LiveArea")
    require(chunks[-1] == (b"IEND", b"") and kinds.count(b"IEND") == 1, "invalid PNG ending")
    pixels = zlib.decompress(b"".join(body for kind, body in chunks if kind == b"IDAT"))
    require(len(pixels) == height * (width + 1), "incorrect decompressed image length")
    require(all(pixels[y * (width + 1)] <= 4 for y in range(height)), "invalid PNG row filter")
    return {"width": width, "height": height, "bit_depth": depth, "color_type": color, "opaque": True}


def check_archive(archive):
    results = {name: check_png(archive.read(name), dimensions, name) for name, dimensions in IMAGES.items()}
    template = ET.fromstring(archive.read("sce_sys/livearea/contents/template.xml"))
    if template.tag != "livearea" or template.attrib.get("style") != "a1":
        raise ValueError("Unexpected LiveArea template")
    for element, filename in [("livearea-background/image", "bg.png"), ("gate/startup-image", "startup.png")]:
        if template.findtext(element) != filename:
            raise ValueError(f"LiveArea template must reference {filename}")
    return results
