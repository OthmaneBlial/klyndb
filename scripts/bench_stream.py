#!/usr/bin/env python3
"""Record real release backend streaming results; does not measure desktop UI."""
import argparse
import hashlib
import json
import platform
import re
import statistics
import subprocess
from datetime import datetime, timezone
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runs', type=int, default=5)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if not 1 <= args.runs <= 50:
    parser.error('--runs must be 1–50')
root = Path(__file__).resolve().parents[1]
binary = root / 'target/release/examples/stream_bench'
if not binary.is_file():
    parser.error('Build first: cargo build --release -p klyndb-core --example stream_bench')
system = platform.system()
if system not in ('Darwin', 'Linux'):
    parser.error('This measurement runner supports macOS and Linux')
result = {
    'recorded_at': datetime.now(timezone.utc).isoformat(),
    'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
    'cargo_lock_sha256': hashlib.sha256((root / 'Cargo.lock').read_bytes()).hexdigest(),
    'os': platform.platform(),
    'architecture': platform.machine(),
    'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(),
    'scope': 'Release SQLite recursive cursor -> bounded channel -> disk spool -> last 500-row page; includes temp database setup in peak process RSS. No desktop/WebView or comparative client measurements.',
    'samples': [],
    'medians': {},
}
if system == 'Darwin':
    result['cpu'] = subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string'], text=True).strip()
for rows in (100000, 1000000):
    samples = []
    for _ in range(args.runs):
        measured = subprocess.run(['/usr/bin/time', '-l' if system == 'Darwin' else '-v', str(binary), str(rows)], cwd=root, capture_output=True, text=True, check=True)
        sample = json.loads(measured.stdout)
        assert sample['rows'] == rows
        rss = re.search(r'(\d+)\s+maximum resident set size', measured.stderr) if system == 'Darwin' else re.search(r'Maximum resident set size \(kbytes\):\s*(\d+)', measured.stderr)
        if not rss:
            raise RuntimeError('Peak RSS was not reported by the native time command')
        sample['peak_rss_bytes'] = int(rss.group(1)) * (1 if system == 'Darwin' else 1024)
        samples.append(sample)
        print(json.dumps(sample), flush=True)
    result['samples'].extend(samples)
    result['medians'][str(rows)] = {key: statistics.median(s[key] for s in samples) for key in ('elapsed_ms', 'rows_per_second', 'first_row_ms', 'last_page_us', 'peak_rss_bytes')}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, indent=2) + '\n')
print(f'Wrote {args.output}')
