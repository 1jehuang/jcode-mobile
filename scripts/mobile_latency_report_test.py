#!/usr/bin/env python3
"""Offline synthetic tests. No credentials, network calls or device needed."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import mobile_latency_report as latency


def payload(events):
    return json.dumps({'owner': 'private', 'batches': [
        {'run_id': 'run-a', 'batch_id': 'private', 'events': [
            dict(seq=i, kind='timing', name=name, duration_ms=value)
            for i, (name, value) in enumerate(events)]}]})


class ReportTests(unittest.TestCase):
    def test_percentiles_and_ranking(self):
        result = latency.report(payload([('direct_ssh', i) for i in range(1, 101)]
                                        + [('api_hello', 200)]))
        self.assertEqual(result['latency'][0]['name'], 'api_hello')
        self.assertEqual(result['latency'][1], dict(name='direct_ssh', count=100,
                         p50_ms=50, p95_ms=95, max_ms=100))

    def test_lifetime_is_separate(self):
        result = latency.report(payload([('session_lost_after', 500000), ('session_ready', 12)]))
        self.assertEqual(result['latency'][0]['max_ms'], 12)
        self.assertEqual(result['lifetime'][0]['max_ms'], 500000)

    def test_privacy_and_invalid_values(self):
        events = [('private-user-message', 999), ('direct_ssh', True),
                  ('direct_ssh', -1), ('direct_ssh', float('nan')),
                  ('direct_ssh', float('inf')), ('direct_ssh', 604800001),
                  (['secret'], 5), ('direct_ssh', 10 ** 1000), ('direct_ssh', 0)]
        result = latency.report(payload(events))
        self.assertEqual(result['ignored_samples'], 8)
        self.assertEqual(result['latency'][0]['count'], 1)
        self.assertNotIn('private', json.dumps(result))
        self.assertNotIn('secret', json.dumps(result))

    def test_retries_and_independent_runs(self):
        data = json.loads(payload([('direct_ssh', 4)]))
        batch = data['batches'][0]
        data['batches'].append(batch)
        data['batches'].append(dict(batch, run_id='run-b'))
        self.assertEqual(latency.report(json.dumps(data))['latency'][0]['count'], 2)

    def test_log_allowlist_and_no_raw_lines(self):
        text = ('user content\nINFO conn_timing direct ssh 12.5ms\n'
                'INFO conn_timing session lost after 9999.0ms\n'
                'INFO conn_timing private-token 500.0ms\n'
                'INFO conn_event session fatal private-error\n')
        result = latency.report(text, log=True)
        self.assertEqual(result['latency'][0]['p95_ms'], 12.5)
        self.assertEqual(result['ignored_samples'], 1)
        self.assertNotIn('private', json.dumps(result))

    def test_empty_queue(self):
        self.assertEqual(latency.report('{"batches": []}'),
                         {'latency': [], 'lifetime': [], 'ignored_samples': 0})

    def test_bad_container(self):
        for text in ('[]', '{}', '{"batches": [{"events": null}]}'):
            with self.assertRaises(ValueError):
                latency.report(text)

    def test_cli_error_does_not_echo_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'private-secret.json'
            path.write_text('private-user-message')
            process = subprocess.run([sys.executable, str(Path(latency.__file__)), str(path)],
                                     text=True, capture_output=True)
            self.assertEqual(process.returncode, 2)
            self.assertNotIn('private', process.stderr)
            self.assertEqual(process.stdout, '')


if __name__ == '__main__':
    unittest.main()
