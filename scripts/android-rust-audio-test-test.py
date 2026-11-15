#!/usr/bin/env python3
"""Synthetic runner control-flow tests only. Never executes real adb or a device."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
HOOK = 'Java_org_jcode_mobile_rustaudiotests_RustAudioInstrumentation_nativeRun'
PACKAGE = 'org.jcode.mobile.rustaudiotests'


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rust-audio-runner-test-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.root = Path(self.temp.name)
        self.sdk = self.root / 'sdk'
        self.log = self.root / 'commands.jsonl'
        self.env = dict(os.environ, ANDROID_HOME=str(self.sdk), ANDROID_NDK_HOME=str(self.root / 'ndk'),
                        ANDROID_RUST_AUDIO_TEST_OUT=str(self.root / 'out'), SYNTHETIC_LOG=str(self.log),
                        SYNTHETIC_ROOT=str(self.root), SYNTHETIC_MODE='good')
        self.runner = self.root / 'scripts/android-rust-audio-test.sh'
        self.runner.parent.mkdir()
        shutil.copyfile(ROOT / 'scripts/android-rust-audio-test.sh', self.runner)
        shutil.copyfile(ROOT / 'scripts/android-instrumentation-result.py', self.runner.parent / 'android-instrumentation-result.py')
        self.executable(self.root / 'android/rust-audio-tests/build.sh', '#!/bin/sh\nmkdir -p "$ANDROID_RUST_AUDIO_TEST_OUT"\n: > "$ANDROID_RUST_AUDIO_TEST_OUT/rust-audio-tests.apk"\n')
        with zipfile.ZipFile(self.root / 'production.apk', 'w') as archive:
            archive.writestr('lib/x86_64/libjcode_mobile.so', b'SYNTHETIC NOT ELF')
        self.executable(self.sdk / 'platform-tools/adb', '''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys, time
args = sys.argv[3:]
with open(os.environ['SYNTHETIC_LOG'], 'a') as log: log.write(json.dumps(args) + '\\n')
mode = os.environ['SYNTHETIC_MODE']
if args[:3] == ['shell','getprop','ro.kernel.qemu']: print('0' if mode == 'physical' else '1')
elif args[:3] == ['shell','getprop','ro.product.cpu.abilist']: print('x86' if mode == 'abi' else 'x86_64,x86')
elif args[:3] == ['shell','pm','path']: print('package:/data/app/base.apk')
elif args[0] == 'pull': shutil.copyfile(pathlib.Path(os.environ['SYNTHETIC_ROOT']) / 'production.apk', args[2])
elif args[:3] == ['shell','am','instrument']:
    if mode == 'hang':
        pathlib.Path(os.environ['SYNTHETIC_ROOT'], 'instrument-running').touch()
        time.sleep(30)
    if mode != 'missing':
        print('INSTRUMENTATION_RESULT: result=PASS')
        print('INSTRUMENTATION_CODE: -1')
    if mode == 'duplicate': print('INSTRUMENTATION_RESULT: result=PASS')
    if mode in ('failed', 'failed-cleanup'): sys.exit(42)
    if mode == 'timeout': sys.exit(124)
elif args[:3] == ['shell','pm','revoke'] and mode in ('cleanup', 'failed-cleanup'): sys.exit(9)
''')
        self.executable(self.sdk / 'build-tools/35.0.0/aapt2', '''#!/usr/bin/env python3
import os
print("package: name='org.jcode.mobile' versionCode='1'")
if os.environ['SYNTHETIC_MODE'] != 'release': print('application-debuggable')
''')
        self.executable(self.root / 'ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf', f'''#!/usr/bin/env python3
import os
if os.environ['SYNTHETIC_MODE'] != 'hook': print('12: 0000 200 FUNC GLOBAL DEFAULT 15 {HOOK}')
''')

    def tearDown(self):
        self.temp.cleanup()

    def executable(self, path, content):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        path.chmod(0o755)

    def commands(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def run_runner(self, mode='good', args=('--run', 'emulator-synthetic')):
        self.env['SYNTHETIC_MODE'] = mode
        return subprocess.run(['bash', str(self.runner), *args], env=self.env, capture_output=True, text=True, timeout=15)

    def assert_cleanup(self):
        commands = self.commands()
        self.assertIn(['shell', 'am', 'force-stop', PACKAGE], commands)
        self.assertIn(['shell', 'pm', 'revoke', PACKAGE, 'android.permission.RECORD_AUDIO'], commands)
        # Every mutation is restricted to the isolated test UID.
        for command in commands:
            if command[:3] in (['shell', 'pm', 'grant'], ['shell', 'pm', 'revoke'], ['shell', 'am', 'force-stop']):
                self.assertEqual(command[3], PACKAGE)
        self.assertFalse(list((self.root / 'out').glob('preflight.*')))

    def test_build_only_never_invokes_adb(self):
        self.assertEqual(self.run_runner(args=()).returncode, 0)
        self.assertEqual(self.commands(), [])

    def test_invalid_invocation_never_invokes_adb(self):
        self.assertEqual(self.run_runner(args=('--run', 'physical-device')).returncode, 2)
        self.assertEqual(self.commands(), [])

    def test_preflight_refuses_before_any_install_or_grant(self):
        for mode in ('physical', 'release', 'hook', 'abi'):
            with self.subTest(mode=mode):
                self.log.unlink(missing_ok=True)
                self.assertNotEqual(self.run_runner(mode).returncode, 0)
                self.assertFalse(any(c[0] == 'install' or c[:3] == ['shell', 'pm', 'grant'] for c in self.commands()))
                self.assertFalse(list((self.root / 'out').glob('preflight.*')))

    def test_success_and_cleanup(self):
        result = self.run_runner()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_cleanup()
        commands = self.commands()
        self.assertLess(next(i for i, c in enumerate(commands) if c[0] == 'pull'), next(i for i, c in enumerate(commands) if c[:3] == ['shell', 'pm', 'grant']))

    def test_missing_or_duplicate_terminal_result_fails(self):
        for mode in ('missing', 'duplicate'):
            with self.subTest(mode=mode):
                self.assertNotEqual(self.run_runner(mode).returncode, 0)
                self.assert_cleanup()

    def test_failed_instrumentation_and_timeout_are_not_hidden_by_pass_text(self):
        for mode, code in (('failed', 42), ('timeout', 124), ('failed-cleanup', 42)):
            with self.subTest(mode=mode):
                self.assertEqual(self.run_runner(mode).returncode, code)
                self.assert_cleanup()

    def test_cleanup_failure_turns_success_into_failure(self):
        self.assertEqual(self.run_runner('cleanup').returncode, 1)
        self.assert_cleanup()

    def test_signals_preserve_failure_and_cleanup(self):
        for sig, expected in ((signal.SIGINT, 130), (signal.SIGTERM, 143)):
            with self.subTest(signal=sig):
                self.log.unlink(missing_ok=True)
                marker = self.root / 'instrument-running'
                marker.unlink(missing_ok=True)
                self.env['SYNTHETIC_MODE'] = 'hang'
                process = subprocess.Popen(['bash', str(self.runner), '--run', 'emulator-synthetic'], env=self.env,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                try:
                    deadline = time.monotonic() + 5
                    while not marker.exists() and time.monotonic() < deadline:
                        time.sleep(0.01)
                    self.assertTrue(marker.exists(), 'Synthetic instrument did not start')
                    process.send_signal(sig)
                    process.communicate(timeout=5)
                    self.assertEqual(process.returncode, expected)
                    self.assert_cleanup()
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.communicate()


if __name__ == '__main__':
    unittest.main()
