#!/usr/bin/env python3
"""Summarize retained metadata only; never infer input-to-photon latency."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
from analyze_latency import describe, history_summary, trace_summary


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--latest', type=Path, required=True)
    parser.add_argument('--historical', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = {'hardware_after_measurements': None, 'histories': [], 'traces': [], 'files': []}
    seen = set()
    for folder, latest in [(args.latest, True), (args.historical, False)]:
        for path in sorted(folder.rglob('pipeline-*')):
            if path.suffix not in ('.txt', '.csv') or not path.is_file():
                continue
            data = path.read_bytes()
            sha = hashlib.sha256(data).hexdigest()
            result['files'].append({'name': path.name, 'latest_attachment': latest,
                                    'bytes': len(data), 'sha256': sha})
            if sha in seen:
                continue
            seen.add(sha)
            if 'history' in path.name:
                report = history_summary(data.decode())
                samples = report['samples']
                fields = ('queue_depth', 'video_arrival_growth_ms', 'audio_arrival_growth_ms',
                          'decoder_residence_avg_ms', 'decoder_residence_max_ms',
                          'receive_to_gpu_avg_ms', 'receive_to_gpu_max_ms', 'audio_sdl_ms')
                result['histories'].append({'name': path.name, 'latest_attachment': latest,
                    'builds': sorted({s['build'] or 'unknown' for s in samples}),
                    'retained_seconds': [samples[0]['elapsed_ms']/1000, samples[-1]['elapsed_ms']/1000] if samples else [],
                    'windows': len(samples), 'distributions_of_window_values': {
                        f: describe([s['metrics'][f] for s in samples if s['metrics'][f] is not None]) for f in fields}})
                if latest:
                    result['latest_history_samples'] = samples
            elif 'trace' in path.name:
                rows = list(csv.DictReader(data.decode().splitlines()))
                if rows and 'stage' in rows[0] and 'value_us_or_reason' in rows[0]:
                    summary = trace_summary(rows)
                    summary.pop('decoder_output_sequence', None)
                    result['traces'].append({'name': path.name, 'latest_attachment': latest, **summary})
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    for item in result['histories']:
        d = item['distributions_of_window_values']
        print(item['name'], item['builds'], item['windows'],
              'video growth max', (d['video_arrival_growth_ms'] or {}).get('max'),
              'receive/GPU max', (d['receive_to_gpu_max_ms'] or {}).get('max'))


if __name__ == '__main__':
    main()
