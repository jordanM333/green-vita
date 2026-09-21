import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("analysis", Path(__file__).parents[2] / "tools/analyze_latency.py")
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class EvidenceTests(unittest.TestCase):
    def test_fast_decode_and_empty_queue_do_not_prove_responsiveness(self):
        rows = [{"elapsed_us": "10000000", "stage": "decode_submit", "value_us_or_reason": "0"},
                {"elapsed_us": "10002000", "stage": "decode_return", "value_us_or_reason": "2000"},
                {"elapsed_us": "10003000", "stage": "picture_generation", "value_us_or_reason": "99999999"}]
        result = analysis.trace_summary(rows)
        self.assertIsNone(result["end_to_end_latency_ms"])
        self.assertIsNone(result["local_measurements"]["matched_decoder_residence_ms"])
        self.assertEqual(result["local_measurements"]["decode_call_ms"]["max"], 2)
        self.assertEqual(result["retained_span_s"], .003)

    def test_unsynchronized_offsets_are_not_latency(self):
        report = analysis.history_summary("elapsed_ms=1000\nSR offset* V:6000ms A:6000ms\n")
        self.assertIsNone(report["end_to_end_latency_ms"])

    def test_mixed_session_clocks_are_rejected(self):
        with self.assertRaises(ValueError):
            analysis.history_summary("elapsed_ms=2000\na\nelapsed_ms=1000\nb")
        with self.assertRaises(ValueError):
            analysis.trace_summary([{"elapsed_us": "2"}, {"elapsed_us": "1"}])

    def camera(self, late, uncertainty=8):
        return [{"run": "r1", "mode": "Home", "phase": phase, "trial": str(i),
                 "press_s": "1", "tv_s": "1.1", "vita_s": str(1 + latency / 1000),
                 "uncertainty_ms": str(uncertainty)}
                for phase, latency in [("early", 200), ("late", late)] for i in range(5)]

    def test_camera_separates_tv_response_from_extra_vita_delay(self):
        report = analysis.camera_summary(self.camera(5200))
        self.assertAlmostEqual(report["trials"][-1]["button_to_tv"]["estimate_ms"], 100)
        self.assertAlmostEqual(report["trials"][-1]["vita_minus_tv"]["estimate_ms"], 5100)
        self.assertEqual(report["comparisons"][0]["drift_assessment"], "exceeds_100ms_bound")

    def test_uncertain_threshold_does_not_false_pass(self):
        report = analysis.camera_summary(self.camera(270, uncertainty=17))
        self.assertEqual(report["comparisons"][0]["drift_assessment"], "inconclusive_at_100ms")

    def test_stable_sluggish_start_is_never_a_complete_pass(self):
        rows = self.camera(200)
        for row in rows:
            row["vita_s"] = "6"
        report = analysis.camera_summary(rows)
        self.assertEqual(report["comparisons"][0]["drift_assessment"], "within_100ms_bound")
        self.assertEqual(report["comparisons"][0]["phases"]["early"]["median"], 5000)
        self.assertEqual(report["acceptance"], "not_established")

    def test_recovery_trials_do_not_replace_stable_network_late_trials(self):
        rows = self.camera(200)
        for row in rows:
            if row["phase"] == "late": row["phase"] = "network_recovery"
        report = analysis.camera_summary(rows)
        self.assertEqual(report["comparisons"][0]["drift_assessment"], "insufficient_trials")

    def test_invalid_or_duplicate_camera_evidence_is_rejected(self):
        rows = self.camera(200)
        with self.assertRaises(ValueError): analysis.camera_summary(rows + rows[:1])
        rows[0]["uncertainty_ms"] = "nan"
        with self.assertRaises(ValueError): analysis.camera_summary(rows)


if __name__ == "__main__": unittest.main()
