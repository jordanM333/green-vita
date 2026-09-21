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

    def test_test34_startup_window_is_not_current_session_decoder_baseline(self):
        report = analysis.history_summary(
            "elapsed_ms=5455\nBuild: RX Test 34 revision same\n"
            "H264: waiting for SPS\nRTP pk:0\nFrame age: decoder:857/888ms receiveToGPU:875/902ms\n"
            "elapsed_ms=8460\nBuild: RX Test 34 revision same\n"
            "RTP pk:264\nFrame age: decoder:69/85ms receiveToGPU:91/100ms\n"
            "elapsed_ms=61539\nBuild: RX Test 34 revision same\n"
            "RTP pk:7631\nFrame age: decoder:620/639ms receiveToGPU:638/647ms\n")
        self.assertTrue(report["samples"][0]["metrics"]["startup_or_inherited_window"])
        self.assertEqual(report["local_decoder_window_change"]["change_in_window_average_ms"], 551)
        self.assertIsNone(report["end_to_end_latency_ms"])

    def test_no_output_window_is_not_zero_residence_and_clocks_are_separate(self):
        metrics = analysis.history_metrics(
            "RTP pk:1307\nFrame age: decoder:n/a receiveToGPU:n/a\n"
            "FPS hwCall:0 decoded:0 shown:0 ui:4\n"
            "Link ICE:11ms/? DCbuf:1/47 | SR offset* V:16738ms d+24 rel+27ms SR:26/0s "
            "A:16793ms d+77 rel+82ms SR:1/2s V-A:-55ms\n"
            "Recovery wait:1595ms max:1595ms completed:0\n")
        self.assertIsNone(metrics["decoder_residence_avg_ms"])
        self.assertEqual(metrics["decoded"], 0)
        self.assertEqual(metrics["video_arrival_growth_ms"], 27)
        self.assertEqual(metrics["audio_arrival_growth_ms"], 82)
        self.assertEqual(metrics["recovery_wait_ms"], 1595)

    def trace_row(self, time, stage, rtp, value):
        return dict(elapsed_us=str(time), stage=stage,
                    submission_rtp_timestamp=str(rtp), value_us_or_reason=str(value))

    def test_decoder_output_uses_returned_pts_instead_of_current_submission(self):
        row = self.trace_row
        result = analysis.decoder_output_sequence([
            row(1000, "decode_submit", 1000, 100),
            row(621000, "decode_submit", 56800, 200),
            row(623000, "decoder_output_pts", 56800, 1000),
            row(623010, "old_picture_epoch", 1000, 3)])
        output = result["outputs"][0]
        self.assertEqual(output["submit_to_output_ms"], 622)
        self.assertEqual(output["rtp_lead_ms"], 620)
        self.assertEqual(output["current_submission_receive_age_ms"], .2)
        self.assertEqual(output["returned_rtp"], 1000)
        # Raw returned pictures count even if publication subsequently rejects the epoch.
        self.assertEqual(len(result["outputs"]), 1)

    def test_trace_ring_boundary_unknown_pts_and_repeated_outputs_do_not_invent_associations(self):
        row = self.trace_row
        result = analysis.decoder_output_sequence([
            row(1000, "decoder_output_pts", 900, 100),
            row(2000, "decode_submit", 1000, 100),
            row(3000, "decoder_output_pts", 1000, (1 << 64) - 1),
            row(4000, "decoder_output_pts", 1000, 1000),
            row(5000, "decoder_output_pts", 1000, 1000)])
        outputs = result["outputs"]
        self.assertEqual(outputs[0]["association"], "submission_not_retained")
        self.assertIsNone(outputs[0]["submit_to_output_ms"])
        self.assertEqual(outputs[1]["association"], "unknown_pts")
        self.assertTrue(outputs[3]["duplicate_returned_pts"])

    def test_returned_extended_pts_wrap_is_not_a_multi_hour_lead(self):
        row = self.trace_row
        result = analysis.decoder_output_sequence([
            row(1000, "decode_submit", 0xfffffa24, 0),
            row(21000, "decoder_output_pts", 0, 0xfffffa24),
            row(23000, "decode_submit", 0, 0),
            row(25000, "decoder_output_pts", 1500, 1 << 32)])
        self.assertAlmostEqual(result["outputs"][0]["rtp_lead_ms"], 1500 / 90)
        self.assertEqual(result["outputs"][1]["returned_rtp"], 0)
        self.assertEqual(result["outputs"][1]["submit_to_output_ms"], 2)

    def test_reused_rtp_identity_is_ambiguous_instead_of_guessed(self):
        row = self.trace_row
        result = analysis.decoder_output_sequence([
            row(1000, "decode_submit", 100, 0),
            row(2000, "decode_submit", 100, 0),
            row(3000, "decoder_output_pts", 100, 100)])
        self.assertEqual(result["outputs"][0]["association"], "ambiguous_submission")
        self.assertIsNone(result["outputs"][0]["submit_to_output_ms"])

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
