#!/usr/bin/env python3
"""Fetch and symbolize jcode-mobile crashes.

  scripts/crash_fetch.py                 latest TestFlight crash submissions (default 3)
  scripts/crash_fetch.py -n 10           more
  scripts/crash_fetch.py --id ANvTpbI5_JIKemVKYJwBjA4
  scripts/crash_fetch.py --backend       latest panic reports from D1 mobile_crashes
  scripts/crash_fetch.py --file crash.txt [--binary path]   symbolize a local log

Binaries come from ~/.appstoreconnect/builds/<CFBundleVersion>/jcode-mobile
(scripts/ios-bundle.sh --testflight archives them). Symbolization is a
nearest-preceding-symbol lookup over `llvm-nm -n` (llvm-symbolizer fails on
these Mach-O binaries). --binary forces a binary even if the UUID differs
(results are then approximate at best).
"""
import argparse
import bisect
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

APP_ID = "6830033634"
IMAGE = "jcode-mobile"
BUILDS = os.path.expanduser("~/.appstoreconnect/builds")
BACKEND = os.path.expanduser("~/solosystems-backend/workers/api")
TEXT_VMADDR = 0x100000000

FRAME_RE = re.compile(r"^(\d+)\s+(\S+)\s+\t?(0x[0-9a-f]+)\s+(.*)$")
IMAGE_RE = re.compile(r"^\s*(0x[0-9a-f]+)\s+-\s+0x[0-9a-f]+\s+(\S+)\s+\S+\s+<([0-9a-f]{32})>")
VERSION_RE = re.compile(r"^Version:\s+\S+\s+\((\d+)\)", re.M)


def norm_uuid(u):
    return u.replace("-", "").lower()


def binary_uuid(path):
    out = subprocess.run(["llvm-objdump", "--macho", "--private-headers", path],
                         capture_output=True, text=True).stdout
    m = re.search(r"uuid\s+([0-9A-Fa-f-]{36})", out)
    return norm_uuid(m.group(1)) if m else None


class Symbols:
    def __init__(self, path):
        out = subprocess.run(["llvm-nm", "-n", "--demangle", "--defined-only", path],
                             capture_output=True, text=True, check=True).stdout
        self.addrs, self.names = [], []
        for line in out.splitlines():
            parts = line.split(" ", 2)
            if len(parts) == 3 and parts[1] in "tT":
                self.addrs.append(int(parts[0], 16))
                self.names.append(parts[2])

    def lookup(self, vmaddr):
        i = bisect.bisect_right(self.addrs, vmaddr) - 1
        if i < 0:
            return None
        return f"{strip_hash(self.names[i])} + {vmaddr - self.addrs[i]}"


def strip_hash(name):
    return re.sub(r"::h[0-9a-f]{16}$", "", name)


def find_binary(build, uuid, override):
    if override:
        return override, binary_uuid(override) == uuid
    cand = os.path.join(BUILDS, str(build), IMAGE) if build else None
    if cand and os.path.exists(cand):
        return cand, binary_uuid(cand) == uuid
    # Fall back to scanning every archive for the UUID.
    if uuid and os.path.isdir(BUILDS):
        for b in sorted(os.listdir(BUILDS), reverse=True):
            p = os.path.join(BUILDS, b, IMAGE)
            if os.path.exists(p) and binary_uuid(p) == uuid:
                return p, True
    return None, False


def symbolize_apple_log(text, override=None):
    """Annotate an Apple .ips-style crash log. Returns the rendered text."""
    base = uuid = None
    for line in text.splitlines():
        m = IMAGE_RE.match(line)
        if m and m.group(2) == IMAGE:
            base, uuid = int(m.group(1), 16), m.group(3)
            break
    vm = VERSION_RE.search(text)
    build = vm.group(1) if vm else None
    out = [f"build {build}  image base {hex(base) if base else '?'}  uuid {uuid}"]
    path, exact = find_binary(build, uuid, override)
    if not path:
        out.append(f"!! no archived binary for build {build} (looked in {BUILDS}); raw frames only")
    elif not exact:
        out.append(f"!! {path} UUID {binary_uuid(path)} != crash UUID {uuid}: symbols are a GUESS")
    syms = Symbols(path) if path else None
    in_crashed = False
    for line in text.splitlines():
        if re.match(r"^Thread \d+ Crashed", line):
            in_crashed = True
            out.append(line)
            continue
        if in_crashed and not line.strip():
            break
        if not in_crashed:
            continue
        m = FRAME_RE.match(line)
        if not m:
            continue
        idx, img, addr, rest = m.groups()
        if img == IMAGE and syms and base is not None:
            name = syms.lookup(TEXT_VMADDR + int(addr, 16) - base)
            out.append(f"{idx:>3} {name or rest}")
        else:
            out.append(f"{idx:>3} [{img}] {rest}")
    return "\n".join(out)


def symbolize_panic_report(report, override=None):
    """Annotate a report written by jcode_mobile_core::crash (raw_frames line)."""
    head = report.split("\nbacktrace:", 1)[0]
    field = lambda k: next((l[len(k):].strip() for l in report.splitlines() if l.startswith(k)), None)
    base, raw, build = field("image_base:"), field("raw_frames:"), field("build:")
    out = [head]
    if not (base and raw):
        out.append(report.split("\nbacktrace:", 1)[-1][:4000])
        return "\n".join(out)
    base = int(base, 16)
    path, _ = find_binary(build, None, override)
    syms = Symbols(path) if path else None
    if not syms:
        out.append(f"!! no archived binary for build {build}")
    for i, a in enumerate(raw.split()):
        off = int(a, 16) - base
        name = syms.lookup(TEXT_VMADDR + off) if syms and 0 <= off < 1 << 32 else None
        out.append(f"{i:>3} {name or a}")
    return "\n".join(out)


def testflight(args):
    import asc
    if args.id:
        status, data = asc.call("GET", f"/v1/betaFeedbackCrashSubmissions/{args.id}")
        subs = [data["data"]]
    else:
        status, data = asc.call("GET", f"/v1/apps/{APP_ID}/betaFeedbackCrashSubmissions?sort=-createdDate&limit={args.n}")
        subs = data.get("data", [])
    if status != 200:
        sys.exit(f"ASC HTTP {status}: {json.dumps(data)[:500]}")
    for s in subs:
        a = s["attributes"]
        status, log = asc.call("GET", f"/v1/betaFeedbackCrashSubmissions/{s['id']}/crashLog")
        text = log.get("data", {}).get("attributes", {}).get("logText", "")
        print(f"=== {s['id']}  {a.get('createdDate')}  {a.get('deviceModel')} iOS {a.get('osVersion')}")
        print(f"comment: {a.get('comment')!r}")
        print(symbolize_apple_log(text, args.binary) if text else f"(no crash log, HTTP {status})")
        print()


def backend(args):
    sql = f"SELECT crash_id, created_at, account_id, build, os, report FROM mobile_crashes ORDER BY created_at DESC LIMIT {int(args.n)}"
    r = subprocess.run(["npx", "wrangler", "d1", "execute", "jcode-subscriptions", "--remote", "--json", "--command", sql],
                       cwd=BACKEND, capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stderr or r.stdout)
    for row in json.loads(r.stdout)[0]["results"]:
        print(f"=== {row['crash_id']}  {row['created_at']}  build {row['build']}  {row['os']}  account {row['account_id']}")
        print(symbolize_panic_report(row["report"], args.binary))
        print()


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("-n", type=int, default=3)
    p.add_argument("--id")
    p.add_argument("--backend", action="store_true")
    p.add_argument("--file")
    p.add_argument("--binary", help="force this binary (skips UUID match)")
    args = p.parse_args()
    if args.file:
        text = open(args.file).read()
        print(symbolize_panic_report(text, args.binary) if text.startswith("panic:") else symbolize_apple_log(text, args.binary))
    elif args.backend:
        backend(args)
    else:
        testflight(args)


if __name__ == "__main__":
    main()
