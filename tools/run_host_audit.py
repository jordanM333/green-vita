#!/usr/bin/env python3
"""Run all host harnesses without confusing them with Vita/device validation."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--clippy", action="store_true", help="also run host static analysis")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = []
    def run(name, command):
        start = time.monotonic()
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        duration = time.monotonic() - start
        (args.output / f"{name}.log").write_text(result.stdout)
        counts = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", result.stdout)]
        record = dict(name=name, command=command, exit_code=result.returncode,
                      wall_seconds=round(duration, 3), rust_tests_passed=sum(counts))
        results.append(record)
        print(json.dumps(record), flush=True)
    feedback = str(args.output.resolve() / "feedback-tests")
    run("feedback-compile", ["rustc", "--edition=2024", "--test", "src/api/streaming/rtc/feedback.rs", "-o", feedback])
    if results[-1]["exit_code"] == 0:
        run("feedback", [feedback, "--nocapture"])
    for name in ["rtc-reports", "frame-metadata", "rtc-transport", "frame-signal", "rtp-order",
                 "decoder-pump", "feature-foundations", "voice-codec", "audio-pipeline", "session-lifecycle"]:
        run(name, ["cargo", "test", "--locked", "--target", "x86_64-unknown-linux-gnu",
                   "--manifest-path", f"tests/{name}/Cargo.toml", "--", "--test-threads=1", "--nocapture"])
        if args.clippy:
            run(name + "-clippy", ["cargo", "clippy", "--locked", "--all-targets", "--target",
                "x86_64-unknown-linux-gnu", "--manifest-path", f"tests/{name}/Cargo.toml"])
    for name in ["latency-analysis", "link-layout", "audit-contracts"]:
        run(name, ["python3", "-m", "unittest", "discover", "-s", f"tests/{name}", "-v"])
    report = dict(hardware_validation=False, xbox_sender_emulated=False, results=results)
    (args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(any(item["exit_code"] for item in results))


if __name__ == "__main__":
    main()
