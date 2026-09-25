"""Narrow source wiring guards, not behavioral/device integration tests."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("provenance", ROOT / "tools/build_provenance.py")
provenance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(provenance)


class Contracts(unittest.TestCase):
    def test_scoped_checkout_trust_keeps_revision_and_dirty_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            checkout = Path(directory).resolve()
            env = {**os.environ, "HOME": directory, "GIT_CONFIG_NOSYSTEM": "1"}
            def git(*args, checked=True):
                return subprocess.run(["git", *args], cwd=checkout, env=env,
                                      capture_output=True, text=True, check=checked)
            git("init", "-q")
            (checkout / "tracked").write_text("original\n")
            git("add", "tracked")
            git("-c", "user.name=Audit test", "-c", "user.email=audit@example.invalid",
                "commit", "-qm", "fixture")
            revision = git("rev-parse", "HEAD").stdout
            # Git's own test hook models the container checkout ownership mismatch.
            env["GIT_TEST_ASSUME_DIFFERENT_OWNER"] = "1"
            self.assertNotEqual(git("rev-parse", "HEAD", checked=False).returncode, 0)
            env.update(GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="safe.directory",
                       GIT_CONFIG_VALUE_0=str(checkout))
            self.assertEqual(git("rev-parse", "HEAD").stdout, revision)
            self.assertEqual(git("diff", "--name-only", "HEAD").stdout, "")
            (checkout / "tracked").write_text("changed\n")
            self.assertEqual(git("diff", "--name-only", "HEAD").stdout, "tracked\n")
            env["GIT_CONFIG_VALUE_0"] = str(checkout / "another-checkout")
            self.assertNotEqual(git("rev-parse", "HEAD", checked=False).returncode, 0)

    def test_no_unowned_session_sweep(self):
        for path in ["src/app/entry.rs", "src/app/stream_session/connection.rs"]:
            text = (ROOT / path).read_text()
            self.assertNotIn("cleanup_active_sessions", text)
            self.assertNotIn("get_active_session_paths", text)

    def test_refresh_is_media_only(self):
        text = (ROOT / "src/app/stream_session/playback.rs").read_text()
        refresh = text.split("fn refresh_home_stream", 1)[1].split("async fn stop_stream_with_error", 1)[0]
        self.assertIn("streaming.refresh_video()", refresh)
        for forbidden in [".stop(", "start_stream", "tokio::spawn", "send_gamepad_pulse"]:
            self.assertNotIn(forbidden, refresh)

    def test_chat_answer_refreshes_transport_feedback(self):
        text = (ROOT / "src/api/streaming/rtc/worker.rs").read_text()
        self.assertIn("if session.backend.finish_chat_negotiation", text)
        self.assertIn("session.refresh_negotiated_feedback();", text)

    def test_identity_verifier_rejects_missing_embedded_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            vpk = Path(directory) / "test.vpk"
            with zipfile.ZipFile(vpk, "w") as archive:
                archive.writestr("eboot.bin", b"wrong-revision")
                # Empty valid PSF table; identity validation must fail first.
                archive.writestr("sce_sys/param.sfo", provenance.struct.pack("<4sIIII", b"\x00PSF", 0x101, 20, 20, 0))
            with self.assertRaisesRegex(ValueError, "lacks expected identity"):
                provenance.inspect(vpk, "0123456789abcdef", "38.19")


if __name__ == "__main__":
    unittest.main()
