#!/usr/bin/env python3
"""Offline, fail-closed Android App Bundle release inspection."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import ssl
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET
import zipfile

ANDROID = '{http://schemas.android.com/apk/res/android}'


def run(args):
    result = subprocess.run(args, capture_output=True, text=True, env={**os.environ, 'LC_ALL': 'C'})
    if result.returncode:
        raise ValueError(f'{Path(str(args[0])).name} failed (exit {result.returncode})')
    return result.stdout + result.stderr


def java_tool(name):
    home = os.environ.get('JAVA_HOME')
    tool = str(Path(home) / 'bin' / name) if home else shutil.which(name)
    if not tool or not Path(tool).is_file():
        raise ValueError(f'Missing {name}: set JAVA_HOME or PATH')
    return tool


def bundletool(*args):
    jar = os.environ.get('BUNDLETOOL_JAR')
    if not jar or not Path(jar).is_file():
        raise ValueError('Set BUNDLETOOL_JAR to the official bundletool all JAR')
    return run([java_tool('java'), '-jar', jar, *args])


def elf_alignments(data):
    if data[:5] != b'\x7fELF\x02' or data[5] != 1:
        raise ValueError('Native library must be little-endian ELF64')
    offset = struct.unpack_from('<Q', data, 32)[0]
    size, count = struct.unpack_from('<HH', data, 54)
    values = []
    for index in range(count):
        pos = offset + size * index
        if struct.unpack_from('<I', data, pos)[0] == 1:
            values.append(struct.unpack_from('<Q', data, pos + 48)[0])
    if not values or any(value < 16384 for value in values):
        raise ValueError('Native LOAD segments do not all support 16 KiB pages')
    return values


def inspect(aab, policy):
    aab = Path(aab)
    if aab.suffix != '.aab' or not aab.is_file():
        raise ValueError('A real .aab file is required, not an APK')
    digest = hashlib.sha256(aab.read_bytes()).hexdigest()
    with zipfile.ZipFile(aab) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or any(n.startswith('/') or '..' in Path(n).parts for n in names):
            raise ValueError('Unsafe or duplicate bundle archive entries')
        for required in ('BundleConfig.pb', 'base/manifest/AndroidManifest.xml', 'base/dex/classes.dex'):
            if required not in names:
                raise ValueError(f'Missing genuine bundle entry: {required}')
        native = [n for n in names if '/lib/' in n and n.endswith('.so')]
        if not any('/arm64-v8a/' in n for n in native):
            raise ValueError('Missing arm64-v8a native library')
        for name in native:
            elf_alignments(archive.read(name))
    bundletool('validate', f'--bundle={aab}')
    xml = bundletool('dump', 'manifest', f'--bundle={aab}', '--module=base')
    root = ET.fromstring(xml)
    sdk = root.find('uses-sdk')
    app = root.find('application')
    if sdk is None or app is None:
        raise ValueError('Missing SDK/application manifest declarations')
    actual = {
        'package': root.get('package'),
        'version_code': int(root.get(ANDROID + 'versionCode', '0')),
        'version_name': root.get(ANDROID + 'versionName'),
        'min_sdk': int(sdk.get(ANDROID + 'minSdkVersion', '0')),
        'target_sdk': int(sdk.get(ANDROID + 'targetSdkVersion', '0')),
        'permissions': sorted({n.get(ANDROID + 'name') for n in root if n.tag.startswith('uses-permission')}),
    }
    for field in ('package', 'version_code', 'version_name', 'min_sdk', 'permissions'):
        expected = sorted(policy[field]) if field == 'permissions' else policy[field]
        if actual[field] != expected:
            raise ValueError(f'Manifest {field} differs from reviewed release policy')
    if actual['version_code'] <= 0 or actual['target_sdk'] < max(36, policy['minimum_target_sdk']):
        raise ValueError('Invalid versionCode or target SDK below current mobile Play requirement (36)')
    if app.get(ANDROID + 'debuggable', 'false') != 'false' or app.get(ANDROID + 'testOnly', 'false') != 'false':
        raise ValueError('Debuggable/test-only bundle is not releasable')
    verified = run([java_tool('jarsigner'), '-verify', '-verbose', '-certs', str(aab)])
    if 'jar verified.' not in verified or 'unsigned entries' in verified.lower() or 'jar is unsigned' in verified.lower():
        raise ValueError('Bundle signature missing, invalid, or does not cover every entry')
    certs = run([java_tool('keytool'), '-printcert', '-jarfile', str(aab), '-rfc'])
    pem = re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----', certs, re.S)
    if len(pem) != 1:
        raise ValueError('Expected exactly one upload signer certificate')
    fingerprint = hashlib.sha256(ssl.PEM_cert_to_DER_cert(pem[0])).hexdigest()
    expected = policy.get('upload_certificate_sha256', '').replace(':', '').lower()
    if not re.fullmatch('[0-9a-f]{64}', expected) or fingerprint != expected:
        raise ValueError('Upload signer SHA-256 is absent or does not match reviewed policy')
    # Reject standard Android debug identities, even if someone pins that certificate.
    certificate_text = run([java_tool('keytool'), '-printcert', '-jarfile', str(aab)])
    if 'android debug' in certificate_text.lower():
        raise ValueError('Android debug signing certificate is forbidden')
    return {**actual, 'aab_sha256': digest, 'upload_certificate_sha256': fingerprint, 'native_libraries': native, 'offline_artifact_checks_passed': True, 'published': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('aab')
    parser.add_argument('--policy', default='playstore/release-policy.json')
    args = parser.parse_args()
    try:
        print(json.dumps(inspect(args.aab, json.loads(Path(args.policy).read_text())), indent=2))
    except (ValueError, OSError, KeyError, zipfile.BadZipFile, ET.ParseError, struct.error) as error:
        print(f'NOT READY: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
