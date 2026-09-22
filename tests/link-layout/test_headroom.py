import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location(
    "check_vita_elf", Path(__file__).resolve().parents[2] / "tools/check_vita_elf.py"
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def elf(text_size, data_start):
    ident = b"\x7fELF\x01\x01\x01" + bytes(9)
    header = struct.pack("<16sHHIIIIIHHHHHH", ident, 2, 40, 1,
                         0x81000001, 52, 0, 0, 52, 32, 2, 0, 0, 0)
    text = struct.pack("<IIIIIIII", 1, 0, 0x81000000, 0x81000000,
                       text_size, text_size, 5, 0x1000)
    data = struct.pack("<IIIIIIII", 1, 0, data_start, data_start,
                       0x100, 0x400, 6, 0x1000)
    return header + text + data


class HeadroomTests(unittest.TestCase):
    def test_rx38_failed_layout_has_only_4248_bytes(self):
        self.assertEqual(checker.metadata_gap(elf(0x106ef68, 0x82070000)), 4248)

    def test_reserved_gap(self):
        self.assertGreaterEqual(checker.metadata_gap(elf(0x106ef68, 0x82080000)), 65536)

    def test_rounds_metadata_start_to_four_bytes(self):
        self.assertEqual(checker.metadata_gap(elf(0x1001, 0x81012000)), 69628)

    def test_overlap_is_negative(self):
        self.assertLess(checker.metadata_gap(elf(0x2000, 0x81001000)), 0)

    def test_truncated_headers_rejected(self):
        with self.assertRaises(ValueError):
            checker.metadata_gap(elf(0x1000, 0x81012000)[:-1])

    def test_non_elf_rejected(self):
        with self.assertRaises(ValueError):
            checker.metadata_gap(b"not an ELF")


if __name__ == "__main__":
    unittest.main()
