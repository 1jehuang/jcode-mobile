#!/usr/bin/env python3
"""Run the real desktop mobile preview against a loopback cloud/SSH fixture.

No production login, account, cached transcript or endpoint is used. Delays are
injected server-side. Render markers are CPU render observations, not GPU fences.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import select
import signal
import shutil
import subprocess
import threading
import time
from urllib.parse import urlsplit

try:
    from PIL import ImageGrab
except ImportError:
    ImageGrab = None

SCENARIOS = {
    "awake": ["--cp-ms", "40", "--auth-ms", "20", "--hello-ms", "20", "--attach-ms", "20", "--history-ms", "40"],
    "suspended-peer": ["--cp-ms", "40", "--auth-ms", "20", "--hello-ms", "20", "--attach-ms", "20", "--history-ms", "40"],
    "slow-control": ["--cp-ms", "1500", "--auth-ms", "20", "--hello-ms", "20", "--history-ms", "40"],
    "slow-handshake": ["--cp-ms", "40", "--handshake-ms", "1200", "--auth-ms", "20", "--hello-ms", "20", "--history-ms", "40"],
    "slow-auth": ["--cp-ms", "40", "--auth-ms", "1200", "--hello-ms", "20", "--history-ms", "40"],
    "slow-hello": ["--cp-ms", "40", "--auth-ms", "20", "--hello-ms", "1200", "--history-ms", "40"],
    "slow-history": ["--cp-ms", "40", "--auth-ms", "20", "--history-ms", "1500"],
    "large-history": ["--cp-ms", "40", "--auth-ms", "20", "--history-ms", "40", "--history-messages", "250", "--history-bytes", "4096"],
    "boot-polls": ["--cp-ms", "40", "--boot-polls", "3", "--auth-ms", "20", "--history-ms", "40"],
}
MARKERS = ("session connect", "session ready", "session history", "ui_open_to_history", "ui_history_apply", "ui open to transcript render")
TRACE = re.compile(r"startup_trace (.+) \+([0-9.]+)(ns|µs|us|ms|s)$")


def parse_trace(line):
    match = TRACE.search(line.strip())
    if not match:
        return None
    name, value, unit = match.groups()
    factor = {"ns": 1e-6, "µs": .001, "us": .001, "ms": 1., "s": 1000.}[unit]
    return name, float(value) * factor


def isolated_env(data, display):
    # Never inherit dev hooks that opt into a real account or send work.
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("JCODE_", "NARI_", "ANTHROPIC_", "OPENAI_"))
           and key not in ("WAYLAND_DISPLAY", "ZED_HEADLESS", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY",
                           "http_proxy", "https_proxy", "all_proxy")}
    env.update(NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost", DISPLAY=display, XDG_DATA_HOME=str(data / "data"),
               XDG_CONFIG_HOME=str(data / "config"), XDG_CACHE_HOME=str(data / "cache"),
               XDG_STATE_HOME=str(data / "state"), JCODE_STARTUP_TRACE="1",
               JCODE_MOBILE_FRAMELESS="1", JCODE_MOBILE_SIZE="390x844",
               JCODE_MOBILE_KEYBOARD="0")
    return env


def stop(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def collect(stream, events, log, started):
    with stream, log.open("w", encoding="utf8") as output:
        for line in stream:
            elapsed = (time.monotonic() - started) * 1000
            output.write(line)
            output.flush()
            events.put((elapsed, line))


def wait_render(events, process, deadline):
    samples = {}
    while time.monotonic() < deadline:
        try:
            wall_ms, line = events.get(timeout=.1)
        except queue.Empty:
            if process.poll() is not None:
                raise RuntimeError("simulator exited before fresh transcript render")
            continue
        parsed = parse_trace(line)
        if parsed:
            name, duration = parsed
            if name in MARKERS and (name not in samples or name == "ui open to transcript render"):
                samples[name] = {"duration_ms": duration, "observed_ms": wall_ms}
        if "ui open to transcript render" in samples and all(stage in samples for stage in ("session ready", "session history", "ui_open_to_history")):
            # UI application and render run on the same thread. A cached paint
            # preceding fresh history is not proof of fresh content rendering.
            if samples["ui open to transcript render"]["observed_ms"] >= samples["ui_open_to_history"]["observed_ms"]:
                return samples
    raise TimeoutError("fresh ready/history/render milestones did not all arrive")


def capture(env, destination):
    # This display belongs exclusively to this run, no user windows are captured.
    if ImageGrab is not None:
        # XCB reads our private display directly, avoiding ImageMagick process
        # startup and its OpenMP overhead in the presentation upper bound.
        ImageGrab.grab(xdisplay=env["DISPLAY"]).save(destination)
    else:
        subprocess.run(["import", "-display", env["DISPLAY"], "-window", "root", str(destination)], check=True, timeout=10)


def visible_transcript(env, destination, origin, timeout=10, expected_session=None):
    """Verify immutable captured frames offline, without charging OCR to paint."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        frames = []
        # Capture a short burst before OCR, so a pre-presentation first frame
        # does not add a full OCR invocation to the next observed paint time.
        for index in range(3):
            path = destination.with_name(destination.stem + f".frame{index}.png")
            capture(env, path)
            frames.append((path, (time.monotonic() - origin) * 1000))
            if index != 2:
                time.sleep(.02)
        for path, captured_ms in frames:
            text = subprocess.run(["tesseract", str(path), "stdout", "--psm", "11"],
                                  env={**os.environ, "OMP_THREAD_LIMIT": "1", "OMP_NUM_THREADS": "1"},
                                  capture_output=True, text=True, check=True, timeout=10).stdout
            # Small monospace I is often recognized as T by OCR on Xvfb.
            session_visible = expected_session is None or expected_session.upper() in text.upper()
            if re.search(r"S[IT1L]MULATOR", text.upper()) and "FRESH" in text.upper() and session_visible:
                shutil.copyfile(path, destination)
                return {"frame_capture_upper_bound_ms": captured_ms,
                        "ocr_completed_ms": (time.monotonic() - origin) * 1000}
    raise TimeoutError("screenshot never displayed the fresh fixture transcript")


def new_tab(env):
    # Real raw pointer action on the frameless 390x844 simulator's + button.
    subprocess.run(["xdotool", "mousemove", "358", "71", "click", "1"], env=env, check=True, timeout=5)


def run_case(args, name, root, display):
    folder = root / name
    folder.mkdir(mode=0o700)
    fixture = app = None
    readers = []
    events = queue.Queue()
    try:
        fixture = subprocess.Popen([str(args.fixture), *SCENARIOS[name]], stdout=subprocess.PIPE,
                                   stderr=(folder / "fixture.log").open("w"), text=True)
        ready_lines = queue.Queue()
        def read_ready():
            ready_lines.put(fixture.stdout.readline())
        threading.Thread(target=read_ready, daemon=True).start()
        ready = json.loads(ready_lines.get(timeout=15))
        api_base = ready["api_base"]
        url = urlsplit(api_base)
        if url.scheme != "http" or url.hostname != "127.0.0.1" or not url.port:
            raise ValueError("fixture must advertise an ephemeral IPv4 loopback HTTP endpoint")
        account_dir = folder / "data" / "jcode-mobile"
        account_dir.mkdir(parents=True, mode=0o700)
        account_path = account_dir / "account.json"
        account = ready["account"]
        if account.get("api_base") != api_base or not account.get("email", "").endswith("@example.invalid"):
            raise ValueError("fixture must advertise its isolated fake account")
        account_path.write_text(json.dumps(account))
        account_path.chmod(0o600)
        env = isolated_env(folder, display)
        started = time.monotonic()
        app = subprocess.Popen([str(args.app)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        reader = threading.Thread(target=collect, args=(app.stderr, events, folder / "app.log", started), daemon=True)
        reader.start()
        readers.append(reader)
        cold = wait_render(events, app, started + args.timeout)
        cold_visible = visible_transcript(env, folder / "cold.png", started, expected_session="fixture-1")
        # A real second tab uses the process's actual authenticated-master cache.
        # Drain only completed first-open logs, then trigger a raw UI tap.
        while not events.empty():
            events.get_nowait()
        warm_started = time.monotonic()
        if name == "suspended-peer":
            # Freeze only our fixture process, leaving real TCP sockets open.
            # This simulates an unresponsive peer without touching real hosts.
            fixture.send_signal(signal.SIGSTOP)
            def resume_peer():
                time.sleep(.35)
                if fixture.poll() is None:
                    fixture.send_signal(signal.SIGCONT)
            threading.Thread(target=resume_peer, daemon=True).start()
        new_tab(env)
        warm = wait_render(events, app, warm_started + args.timeout)
        for sample in warm.values():
            sample["observed_ms"] -= (warm_started - started) * 1000
        warm_visible = visible_transcript(env, folder / "warm.png", warm_started, expected_session="fixture-2")
        # Restart the real app using only its own just-created persistent state.
        # There is no process-local SSH master and no synthetic cached UI marker.
        stop(app)
        reader.join(timeout=2)
        events = queue.Queue()
        relaunched = time.monotonic()
        app = subprocess.Popen([str(args.app)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        reader = threading.Thread(target=collect, args=(app.stderr, events, folder / "relaunch.log", relaunched), daemon=True)
        reader.start()
        readers.append(reader)
        relaunch = wait_render(events, app, relaunched + args.timeout)
        relaunch_visible = visible_transcript(env, folder / "relaunch.png", relaunched, expected_session="fixture-3")
        return {"scenario": name, "injected_args": SCENARIOS[name],
                "warm_peer_suspension_ms": 350 if name == "suspended-peer" else 0,
                "cold": cold, "warm_tab": warm,
                "relaunch": relaunch, "relaunch_visible": relaunch_visible,
                "cold_visible": cold_visible, "warm_visible": warm_visible,
                "scope": "real native simulator + real HTTP/SSH + synthetic server delays; not iPhone/AWS"}
    finally:
        stop(app)
        if fixture is not None and fixture.poll() is None:
            fixture.send_signal(signal.SIGCONT)
        stop(fixture)
        for reader in readers:
            reader.join(timeout=2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, default=Path("target/release/jcode-mobile"))
    parser.add_argument("--fixture", type=Path, default=Path("target/debug/examples/mobile_latency_fixture"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenario", choices=["all", *SCENARIOS], default="all")
    parser.add_argument("--timeout", type=float, default=45)
    args = parser.parse_args()
    for attr in ("app", "fixture"):
        path = getattr(args, attr).resolve(strict=True)
        if not os.access(path, os.X_OK):
            parser.error(f"{attr} must be executable")
        setattr(args, attr, path)
    if not 1 <= args.timeout <= 180:
        parser.error("timeout must be between 1 and 180 seconds")
    for tool in ("Xvfb", "xdotool", "tesseract", *(("import",) if ImageGrab is None else ())):
        if not shutil.which(tool):
            parser.error(f"missing {tool}")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    read_fd, write_fd = os.pipe()
    xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "390x844x24", "-nolisten", "tcp"], pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=(args.output / "xvfb.log").open("w"))
    os.close(write_fd)
    try:
        with os.fdopen(read_fd) as display_file:
            if not select.select([display_file], [], [], 10)[0]:
                raise TimeoutError("Xvfb did not allocate a private display within 10 seconds")
            display = ":" + display_file.readline().strip()
        if not re.fullmatch(r":\d+", display):
            raise RuntimeError("Xvfb did not allocate a display")
        manifest = {"capture": "Pillow XCB" if ImageGrab is not None else "ImageMagick import",
                    "app_sha256": hashlib.sha256(args.app.read_bytes()).hexdigest(),
                    "fixture_sha256": hashlib.sha256(args.fixture.read_bytes()).hexdigest(), "results": []}
        for name in SCENARIOS if args.scenario == "all" else [args.scenario]:
            result = run_case(args, name, args.output, display)
            for attr in ("app", "fixture"):
                if hashlib.sha256(getattr(args, attr).read_bytes()).hexdigest() != manifest[attr + "_sha256"]:
                    raise RuntimeError(f"{attr} artifact changed during the campaign, refusing mixed-source results")
            manifest["results"].append(result)
            (args.output / "results.json").write_text(json.dumps(manifest, indent=2) + "\n")
            print(json.dumps(result), flush=True)
    finally:
        stop(xvfb)


if __name__ == "__main__":
    main()
