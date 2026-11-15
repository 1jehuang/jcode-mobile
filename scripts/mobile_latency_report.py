#!/usr/bin/env python3
"""Offline, fixed-label mobile duration aggregation. Never emits raw input."""
import argparse
from collections import defaultdict
import json
import math
from pathlib import Path
import re
import sys

# Fixed instrumentation labels only. Never derive output labels from arbitrary text.
LABELS = set("""app_launched app_fonts_loaded app_first_render app_prewarm
 direct_ssh direct_ssh_failed control_plane_wake_direct_ssh control_plane_ready
 prewarm_direct_ssh master_revalidated master_revalidation_failed ssh_handshake
 ssh_authenticated channel_opened api_hello session_connect session_fork
 session_attach session_ready session_lost_after session_connect_failed
 session_connect_error session_history wake_probe_failed wake_probe_timed_out
 ui_history_apply ui_open_to_history ui_slow_frame_build ui_open_to_transcript_render
 ui_transcript_render wake_probe_succeeded initial_history initial_models
 initial_sessions initial_other""".split())
LABELS.update('request_' + name for name in "message cancel soft_interrupt cancel_soft_interrupts ping history set_model set_reasoning_effort compact rename clear list_sessions".split())
LIFETIMES = {'session_lost_after'}
MAX_MS = 604800000


def normalize(name):
    return name.replace(' + ', '_').replace(' ', '_') if isinstance(name, str) else ''


def percentile(values, fraction):
    """Nearest-rank percentile, including small samples without interpolation."""
    return values[max(0, math.ceil(len(values) * fraction) - 1)]


def samples(text, log=False):
    if log:
        for line in text.splitlines():
            match = re.search(r'\bconn_timing (.+?) ([0-9]+(?:\.[0-9]+)?)ms\s*$', line)
            if match:
                yield normalize(match[1]), float(match[2])
        return
    data = json.loads(text)
    if not isinstance(data, dict) or not isinstance(data.get('batches'), list):
        raise ValueError('expected a diagnostics queue or account-scoped batches response')
    seen = set()
    for batch in data['batches']:
        if not isinstance(batch, dict):
            continue
        events = batch.get('events', [])
        if not isinstance(events, list):
            raise ValueError('invalid events container')
        for event in events:
            if not isinstance(event, dict) or event.get('kind') != 'timing':
                continue
            # Retry batches and overlapping exports must not inflate counts.
            identity = (batch.get('run_id'), event.get('seq'))
            if isinstance(identity[0], str) and type(identity[1]) is int:
                if identity in seen:
                    continue
                seen.add(identity)
            yield event.get('name'), event.get('duration_ms')


def report(text, log=False):
    groups = defaultdict(list)
    ignored = 0
    for name, duration in samples(text, log):
        if (not isinstance(name, str) or name not in LABELS or isinstance(duration, bool)
                or not isinstance(duration, (int, float))
                or not 0 <= duration <= MAX_MS or not math.isfinite(duration)):
            ignored += 1
            continue
        groups[name].append(duration)
    result = {'latency': [], 'lifetime': [], 'ignored_samples': ignored}
    for name, values in groups.items():
        values.sort()
        row = dict(name=name, count=len(values), p50_ms=percentile(values, .5),
                   p95_ms=percentile(values, .95), max_ms=values[-1])
        result['lifetime' if name in LIFETIMES else 'latency'].append(row)
    for section in ('latency', 'lifetime'):
        result[section].sort(key=lambda row: (-row['p95_ms'], -row['max_ms'], row['name']))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', help='local queue, scoped ops response, or log file, read only')
    parser.add_argument('--log', action='store_true', help='parse fixed conn_timing lines only')
    args = parser.parse_args()
    try:
        path = Path(args.input)
        if path.stat().st_size > 16 * 1024 * 1024:
            raise ValueError('input exceeds offline report bound')
        result = report(path.read_text(), args.log)
    except (OSError, ValueError, TypeError, KeyError):
        print('Cannot read valid diagnostics input. No input details emitted.', file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, allow_nan=False))
    return 0


if __name__ == '__main__':
    sys.exit(main())
