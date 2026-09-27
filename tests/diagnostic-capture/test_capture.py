"""The production recorder, filesystem save, delayed desktop export and analyzer."""
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import analyze_diagnostic as analyzer
from export_diagnostic import export
from correlate_observer import correlate


def header_pcap(packets):
    """Synthetic wire observation, not an Xbox/wire replay."""
    data = struct.pack("<IHHIIII", 0xa1b2c3d4, 2, 4, 0, 0, 128, 1)
    for e, at in sorted(packets, key=lambda p: p[1]):
        rtp = struct.pack("!BBHII", 0x80, 0x66, e["seq"], e["rtp"], e["ssrc"])
        ip = bytes([0x45, 0, 0, 40, 0, 0, 0, 0, 64, 17, 0, 0])+bytes(8)
        frame = bytes(12)+b"\x08\x00"+ip+struct.pack("!HHHH", 100, 200, 20, 0)+rtp
        seconds, us = divmod(round(at), 1_000_000)
        data += struct.pack("<IIII", seconds, us, len(frame), len(frame))+frame
    return data


class CaptureTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = Path(cls.tmp.name)
        binary = cls.root / "capture-tests"
        subprocess.run(["rustc", "--edition=2024", "-O", "--test", str(ROOT / "tests/diagnostic-capture/harness.rs"), "-o", str(binary)], check=True)
        env = dict(os.environ, GREENVITA_DIAGNOSTIC_FIXTURES=str(cls.root / "device"))
        started = time.perf_counter()
        subprocess.run([str(binary), "--nocapture"], env=env, check=True)
        print(json.dumps({"diagnostic_host_fixture_seconds": time.perf_counter()-started,
                          "hardware_overhead_measured": False}))

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def exported(self, name):
        target = self.root / f"desktop-{name}"
        shutil.copytree(self.root / "device" / name, target, dirs_exist_ok=True)
        archive = self.root / f"{name}.zip"
        if archive.exists():
            archive.unlink()
        digest = export(target, archive)
        self.assertEqual(digest, hashlib.sha256(archive.read_bytes()).hexdigest())
        self.assertEqual(analyzer.read_bundle(target), analyzer.read_bundle(archive))
        return archive

    def test_full_delayed_export_has_onset_baseline_aftermath_audio_and_feedback(self):
        result = analyzer.analyze(self.exported("upstream"))
        self.assertEqual(result["outcome"], "BOUNDARY_EVIDENCE_OBTAINED")
        video, audio = result["streams"]
        self.assertIn("DELAY_GROWTH_BEFORE_UDP_SOCKET_BOUND", video["findings"])
        self.assertGreater(video["deficit_us"], 1_900_000)
        self.assertLess(audio["deficit_us"], 100)
        self.assertLess(video["baseline"]["dequeue_relative_growth"]["max_us"], 100)
        self.assertLess(video["delayed"]["rtc_residence"]["max_us"], 1000)
        self.assertGreater(result["capture"]["frozen_us"]-result["capture"]["trigger_us"], 9_900_000)
        self.assertEqual(result["coverage"]["truncated_records"], 0)
        self.assertTrue(result["feedback_events"])
        self.assertFalse(result["progressive_latency_fixed"])

    def test_two_seconds_inside_client_are_measured_directly(self):
        result = analyzer.analyze(self.exported("rtc"))
        video = next(s for s in result["streams"] if s["media"] == "video")
        self.assertEqual(video["findings"], ["MEASURED_CLIENT_RTC_RESIDENCE_GROWTH"])
        self.assertLess(abs(video["deficit_us"]), 100)
        self.assertGreater(video["delayed"]["rtc_residence"]["max_us"], 2_000_000)

    def test_large_upper_bound_is_not_a_cause(self):
        result = analyzer.analyze(self.exported("ambiguous"))
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertEqual(result["findings"], [])
        self.assertGreater(result["streams"][0]["delayed"]["socket_residence_upper_bound"]["max_us"], 2_000_000)

    def test_missing_bounds_remain_unknown(self):
        result = analyzer.analyze(self.exported("unknown"))
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertIsNone(result["streams"][0]["delayed"]["socket_residence_upper_bound"])

    def test_no_reproduction_is_not_proof_of_fix(self):
        result = analyzer.analyze(self.exported("healthy"))
        self.assertEqual(result["outcome"], "NOT REPRODUCED DURING CAPTURE")
        self.assertFalse(result["progressive_latency_fixed"])

    def altered(self, name, edit):
        destination = self.root / name
        shutil.copytree(self.root / "device/upstream", destination, dirs_exist_ok=True)
        path = destination / "events.csv"
        rows = list(csv.DictReader(io.StringIO(path.read_text())))
        rows = edit(rows)
        with path.open("w") as out:
            writer = csv.DictWriter(out, fieldnames=rows[0].keys())
            writer.writeheader()
            writer.writerows(rows)
        return analyzer.analyze(destination)

    def test_missing_same_packet_observation_not_replaced_by_neighbor(self):
        result = self.altered("no-udp", lambda rows: [r for r in rows if r["stage"] != "udp"])
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertGreater(result["streams"][0]["missing_udp_observations"], 0)

    def test_same_packet_collision_remains_ambiguous(self):
        def duplicate(rows):
            return [r for row in rows for r in ([row, dict(row)] if row["stage"] == "udp" else [row])]
        result = self.altered("collisions", duplicate)
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertGreater(result["streams"][0]["ambiguous_udp_matches"], 0)

    def test_timestamp_reset_is_not_spliced_into_delay(self):
        def reset(rows):
            for row in rows:
                if row["ssrc"] == "7" and int(row["at_us"]) > 5_000_000:
                    row["rtp"] = str((int(row["rtp"])-10_000_000) & 0xffffffff)
            return rows
        result = self.altered("reset", reset)
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertGreater(result["streams"][0]["timestamp_discontinuities"], 0)

    def test_forward_discontinuity_cannot_invent_a_new_fast_baseline(self):
        def jump(rows):
            for row in rows:
                if row["ssrc"] == "7" and int(row["at_us"]) > 5_000_000:
                    row["rtp"] = str((int(row["rtp"])+9_000_000) & 0xffffffff)
            return rows
        result = self.altered("forward-reset", jump)
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertGreater(result["streams"][0]["timestamp_discontinuities"], 0)

    def test_socket_bound_rounding_is_conservative(self):
        result = analyzer.analyze(self.root / "device/upstream")
        self.assertEqual(result["streams"][0]["delayed"]["socket_residence_upper_bound"]["max_us"], 1501)

    def test_windows_and_boundary_witnesses_preserve_measured_intervals(self):
        result = analyzer.analyze(self.exported("upstream"))
        video = result["streams"][0]
        witness = video["boundary_witnesses"][0]
        self.assertEqual(witness["bound"], witness["dequeue"]-witness["empty_probe_started"]+1)
        self.assertEqual(witness["pre_socket_lower"], witness["dequeue_growth"]-witness["bound"]-1)
        late = [w for w in video["dequeue_windows"] if w["second"] >= 12]
        self.assertTrue(late)
        self.assertGreater(late[-1]["pre_socket_lower"]["min_us"], 1_990_000)
        self.assertLess(late[-1]["rtc_residence"]["max_us"], 1000)

    def observer_file(self, mode):
        events = analyzer.events_from(analyzer.read_bundle(self.root / "device/upstream")["events.csv"])
        first, packets = {}, []
        for e in events:
            if e["stage"] != "rtc":
                continue
            base = first.setdefault(e["ssrc"], e)
            # The SAME device packet observations admit both earlier histories.
            media = ((e["rtp"]-base["rtp"]) & 0xffffffff)*1_000_000/e["c"]
            at = e["a"] if mode == "before_observer" else base["a"]+media
            packets.append((e, at+1_000_000_000_000))
        path = self.root / f"{mode}.pcap"
        path.write_bytes(header_pcap(packets))
        return path, packets

    def test_external_point_distinguishes_two_histories_identical_on_device(self):
        bundle = self.exported("upstream")
        before, _ = self.observer_file("before_observer")
        after, _ = self.observer_file("after_observer")
        a = correlate(bundle, before, "SYNTHETIC egress", 0)
        b = correlate(bundle, after, "SYNTHETIC egress", 0)
        av, aa = a["streams"]
        bv, ba = b["streams"]
        self.assertLess(abs(av["observer_to_dequeue_change_plus_clock_skew"]["max_us"]), 2)
        self.assertGreater(bv["observer_to_dequeue_change_plus_clock_skew"]["max_us"], 1_999_000)
        self.assertLess(abs(aa["observer_to_dequeue_change_plus_clock_skew"]["max_us"]), 2)
        self.assertLess(abs(ba["observer_to_dequeue_change_plus_clock_skew"]["max_us"]), 2)
        self.assertEqual(av["timestamp_discontinuities"], 0)  # Includes RTP/sequence wrap.
        self.assertGreater(av["dequeue_windows"][-1]["observer_vs_nominal_media_change"]["min_us"], 1_999_000)
        self.assertLess(abs(bv["dequeue_windows"][-1]["observer_vs_nominal_media_change"]["max_us"]), 2)
        self.assertEqual(a["outcome"], "CORRELATED_OBSERVATIONS_ONLY")
        self.assertEqual(b["outcome"], "CORRELATED_OBSERVATIONS_ONLY")

    def test_duplicate_external_or_device_identities_remain_inconclusive(self):
        path, packets = self.observer_file("before_observer")
        path.write_bytes(header_pcap(packets+packets))
        r = correlate(self.exported("upstream"), path, "synthetic")
        self.assertEqual(r["outcome"], "INCONCLUSIVE")
        self.assertGreater(r["ambiguous_packet_identities"], 0)
        path.write_bytes(header_pcap(packets))
        def duplicate(rows):
            extra = []
            for row in rows:
                if row["stage"] in ("rtc", "udp"):
                    extra.append(dict(row, epoch="2"))
            return rows+extra
        self.altered("reset-identities", duplicate)
        r = correlate(self.root / "reset-identities", path, "synthetic")
        self.assertEqual(r["outcome"], "INCONCLUSIVE")

    def test_observer_requires_raw_same_packet_and_preserves_unknown_drop_count(self):
        path, _ = self.observer_file("before_observer")
        self.altered("observer-no-udp", lambda rows: [r for r in rows if r["stage"] != "udp"])
        r = correlate(self.root / "observer-no-udp", path, "synthetic")
        self.assertEqual(r["outcome"], "INCONCLUSIVE")
        self.assertGreater(r["unmatched_udp_observations"], 0)
        self.assertIsNone(r["capture_drops"])

    def test_audio_only_external_capture_cannot_resolve_video(self):
        path, packets = self.observer_file("before_observer")
        path.write_bytes(header_pcap([(e, at) for e, at in packets if e["media"] == 2]))
        r = correlate(self.exported("upstream"), path, "synthetic")
        self.assertEqual(r["outcome"], "INCONCLUSIVE")
        self.assertTrue(r["streams"])
        self.assertTrue(all(s["stream"][1] == 2 for s in r["streams"]))

    def test_observer_clock_step_is_not_automatically_called_network_delay(self):
        path, packets = self.observer_file("before_observer")
        path.write_bytes(header_pcap([(e, at-(2_000_000 if e["a"] > 12_000_000 else 0)) for e, at in packets]))
        r = correlate(self.exported("upstream"), path, "synthetic")
        self.assertIn("clock skew/steps", r["limitations"])
        for stream in r["streams"]:
            self.assertGreater(stream["observer_to_dequeue_change_plus_clock_skew"]["max_us"], 1_999_000)
        self.assertEqual(r["outcome"], "CORRELATED_OBSERVATIONS_ONLY")

    def test_stream_restart_does_not_cross_correlate(self):
        def reset(rows):
            for row in rows:
                if row["stage"] == "rtc":
                    row["epoch"] = "2"
            return rows
        result = self.altered("epoch", reset)
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertEqual(result["streams"][0]["uniquely_correlated"], 0)

    def test_same_timestamp_duplicate_and_late_frame_do_not_make_clock_deficit(self):
        def duplicate(rows):
            extra = []
            for row in rows:
                if row["stage"] == "rtc" and row["media"] == "1":
                    late = dict(row)
                    late["rtp"] = str((int(row["rtp"])-1500) & 0xffffffff)
                    extra.extend([dict(row), late])
            return rows+extra
        result = self.altered("late", duplicate)
        video = result["streams"][0]
        self.assertGreater(video["same_timestamp_fragments_or_duplicates"], 0)
        self.assertGreater(video["reordered_timestamps"], 0)
        self.assertEqual(video["timestamp_discontinuities"], 0)
        self.assertLess(abs(video["deficit_us"]-2_000_000), 10)


    def test_sender_clock_inconsistency_prevents_attribution(self):
        def add_reports(rows):
            for at, ntp, rtp in [(1_000_000, 1<<32, 0), (6_000_000, 6<<32, 900_000)]:
                row = dict(rows[0])
                row.update(section="onset", stage="sender_report", at_us=str(at), epoch="1", ssrc="7", rtp=str(rtp), seq="0", media="0", flags="0", a=str(ntp), b=str(rtp), c="90000")
                rows.append(row)
            return rows
        result = self.altered("sender-clock", add_reports)
        self.assertEqual(result["outcome"], "INCONCLUSIVE")
        self.assertEqual(result["streams"][0]["sender_clock_check"], "inconsistent")


class ObserverAndClockTest(unittest.TestCase):
    def test_header_only_pcap_parser_and_packet_identity(self):
        import struct
        from correlate_observer import pcap_packets
        rtp = struct.pack("!BBHII", 0x80, 0xe0, 65535, 0xffffffff, 123)
        ip = bytes([0x45,0,0,40,0,0,0,0,64,17,0,0])+bytes(8)
        frame = bytes(12)+b"\x08\x00"+ip+struct.pack("!HHHH", 100, 200, 20, 0)+rtp
        pcap = struct.pack("<IHHIIII",0xa1b2c3d4,2,4,0,0,128,1)
        pcap += struct.pack("<IIII",1,2345,len(frame),len(frame))+frame
        self.assertEqual(list(pcap_packets(pcap)), [((123,65535,0xffffffff),1_002_345)])
        with self.assertRaisesRegex(ValueError,"truncated"):
            list(pcap_packets(pcap[:-1]))
        with self.assertRaisesRegex(ValueError,"classic PCAP"):
            list(pcap_packets(b"\x0a\x0d\x0d\x0a"+bytes(40)))




if __name__ == "__main__":
    unittest.main()
