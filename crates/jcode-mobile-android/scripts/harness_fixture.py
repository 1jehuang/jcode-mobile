#!/usr/bin/env python3
"""Local, deterministic harness API fixture, NOT a real model or cloud server.

Uses only Python's standard library. Frames match jcode-harness-api major 1.
No prompts, credentials, or request bodies are logged. Bind loopback by default.
Run: python3 harness_fixture.py --port 7644
Then: adb reverse tcp:7644 tcp:7644
Android endpoint: tcp://127.0.0.1:7644, token empty.
"""
import argparse
import json
import socketserver
import threading
import time

VERSION = 1
MAX_FRAME = 256 * 1024


class FixtureServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, address, delay=0.03):
        super().__init__(address, Handler)
        self.lock = threading.RLock()
        self.delay = delay
        self.sessions = {
            "fixture-android": {
                "session_id": "fixture-android",
                "title": "Android SDK fixture",
                "status": "idle",
            },
            "fixture-second": {
                "session_id": "fixture-second",
                "title": "Second fixture session",
                "status": "idle",
            },
        }
        self.history = {
            sid: [{"role": "assistant", "content": "Local test fixture. No live model is connected."}]
            for sid in self.sessions
        }

    def handle_error(self, request, client_address):
        # Expected disconnects and malformed input must not print request data.
        pass


class Handler(socketserver.StreamRequestHandler):
    def setup(self):
        super().setup()
        self.session_id = None
        self.hello = False
        self.request.settimeout(60)

    def event(self, event, reply_to=None, **fields):
        frame = {"v": VERSION, "ev": event, **fields}
        if reply_to is not None:
            frame["reply_to"] = reply_to
        self.wfile.write((json.dumps(frame, separators=(",", ":")) + "\n").encode())
        self.wfile.flush()

    def error(self, reply_to, code, message):
        self.event("error", reply_to, code=code, message=message)

    def handle(self):
        try:
            while True:
                line = self.rfile.readline(MAX_FRAME + 1)
                if not line:
                    return
                if len(line) > MAX_FRAME or not line.endswith(b"\n"):
                    return
                try:
                    frame = json.loads(line)
                    if not isinstance(frame, dict):
                        return
                except (ValueError, UnicodeError):
                    return
                self.dispatch(frame)
        except (OSError, ValueError, TypeError):
            return

    def dispatch(self, frame):
        request_id = frame.get("id")
        if not isinstance(request_id, int) or isinstance(request_id, bool):
            return
        if frame.get("v") != VERSION:
            self.error(request_id, "invalid_request", "Unsupported fixture API version")
            return
        action = frame.get("req")
        if action == "hello":
            self.hello = True
            self.event("hello_ok", request_id, version=VERSION,
                       server="android-local-fixture/1", capabilities=[])
            return
        if not self.hello:
            self.error(request_id, "invalid_request", "Send hello first")
            return
        if action == "ping":
            self.event("pong", request_id)
        elif action == "list_sessions":
            with self.server.lock:
                sessions = [dict(item) for item in self.server.sessions.values()]
            self.event("sessions", request_id, sessions=sessions)
        elif action in ("create_session", "attach_session"):
            sid = frame.get("session_id") if action == "attach_session" else "fixture-android"
            with self.server.lock:
                session = self.server.sessions.get(sid)
                if session is not None:
                    session = dict(session)
            if session is None:
                self.error(request_id, "unknown_session", "Unknown fixture session")
                return
            self.session_id = sid
            self.event("attached", request_id, session=session)
            self.event("runtime_info", session_id=sid, provider="local-fixture",
                       model="fixture-not-a-live-model", routes=[])
        elif action == "get_history":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                with self.server.lock:
                    messages = [dict(item) for item in self.server.history[sid]]
                self.event("history", request_id, session_id=sid, messages=messages)
        elif action == "list_models":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                self.event("models", request_id, session_id=sid,
                           models=["fixture-not-a-live-model"], current="fixture-not-a-live-model")
        elif action == "get_runtime_info":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                self.event("runtime_info", request_id, session_id=sid,
                           provider="local-fixture", model="fixture-not-a-live-model", routes=[])
        elif action == "send_message":
            self.send_message(frame, request_id)
        elif action == "cancel":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                self.event("ok", request_id)
                self.event("turn_done", session_id=sid)
        else:
            self.error(request_id, "unknown_request", "Unsupported local fixture request")

    def valid_session(self, frame, request_id):
        sid = frame.get("session_id", self.session_id)
        if not isinstance(sid, str) or sid not in self.server.sessions:
            self.error(request_id, "unknown_session", "Unknown fixture session")
            return None
        return sid

    def send_message(self, frame, request_id):
        sid = self.valid_session(frame, request_id)
        if sid is None:
            return
        text = frame.get("content")
        if not isinstance(text, str) or not text.strip():
            self.error(request_id, "invalid_request", "Fixture needs a nonempty message")
            return
        response = "Local SDK fixture response: your message reached the harness transport. No live model was called."
        with self.server.lock:
            self.server.history[sid].append({"role": "user", "content": text})
            self.server.sessions[sid]["status"] = "processing"
        self.event("message_accepted", request_id, session_id=sid)
        for part in ("Local SDK fixture response: ", "your message reached the harness transport. ", "No live model was called."):
            if self.server.delay:
                time.sleep(self.server.delay)
            self.event("text_delta", session_id=sid, text=part, message_id="fixture-reply")
        self.event("text_done", session_id=sid, message_id="fixture-reply")
        with self.server.lock:
            self.server.history[sid].append({"role": "assistant", "content": response})
            self.server.sessions[sid]["status"] = "idle"
        self.event("turn_done", session_id=sid)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=7644)
    parser.add_argument("--delay", type=float, default=0.03, help="Seconds between streamed chunks")
    args = parser.parse_args()
    if not 0 <= args.port <= 65535 or args.delay < 0:
        parser.error("invalid port or delay")
    with FixtureServer((args.host, args.port), args.delay) as server:
        print(json.dumps({"fixture": "local-sdk-not-live-model", "host": server.server_address[0],
                          "port": server.server_address[1]}), flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
