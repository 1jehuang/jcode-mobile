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
            "s1": {
                "session_id": "s1",
                "title": "Android SDK fixture",
                "status": "idle",
            },
            "s2": {
                "session_id": "s2",
                "title": "Second fixture session",
                "status": "idle",
            },
        }
        self.models = {sid: "fixture-not-a-live-model" for sid in self.sessions}
        self.history = {
            sid: [{"role": "assistant", "content": "earlier"}]
            for sid in self.sessions
        }

    def handle_error(self, request, client_address):
        # Expected disconnects and malformed input must not print request data.
        pass


class Handler(socketserver.StreamRequestHandler):
    def setup(self):
        super().setup()
        self.session_id = None
        self.write_lock = threading.Lock()
        self.active_cancel = None
        self.hello = False
        self.request.settimeout(60)

    def event(self, event, reply_to=None, **fields):
        frame = {"v": VERSION, "ev": event, **fields}
        if reply_to is not None:
            frame["reply_to"] = reply_to
        with self.write_lock:
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
        finally:
            if self.active_cancel is not None:
                self.active_cancel.set()

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
            sid = frame.get("session_id") if action == "attach_session" else "s1"
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
                       model=self.server.models[sid], routes=[])
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
                           models=["fixture-not-a-live-model", "fixture-alternate"], current=self.server.models[sid])
        elif action == "get_runtime_info":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                self.event("runtime_info", request_id, session_id=sid,
                           provider="local-fixture", model=self.server.models[sid], routes=[])
        elif action == "set_model":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                model = frame.get("model")
                if model not in ("fixture-not-a-live-model", "fixture-alternate"):
                    self.error(request_id, "invalid_request", "Unknown fixture model")
                    return
                with self.server.lock:
                    self.server.models[sid] = model
                self.event("ok", request_id)
                self.event("model_info", session_id=sid, provider="local-fixture", model=model)
        elif action == "send_message":
            self.send_message(frame, request_id)
        elif action == "cancel":
            sid = self.valid_session(frame, request_id)
            if sid is not None:
                if self.active_cancel is not None:
                    self.active_cancel.set()
                self.event("ok", request_id)
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
        with self.server.lock:
            if self.server.sessions[sid]["status"] != "idle":
                self.error(request_id, "invalid_request", "Fixture turn is already running")
                return
            self.server.history[sid].append({"role": "user", "content": text})
            self.server.sessions[sid]["status"] = "processing"
        cancel = threading.Event()
        self.active_cancel = cancel
        self.event("message_accepted", request_id, session_id=sid)
        threading.Thread(target=self.stream_reply, args=(sid, cancel, f"fixture-{request_id}"), daemon=True).start()

    def stream_reply(self, sid, cancel, message_id):
        parts = []
        try:
            for part in ("actual ", "streamed ", "reply"):
                if cancel.wait(self.server.delay):
                    break
                self.event("text_delta", session_id=sid, text=part, message_id=message_id)
                parts.append(part)
            self.event("text_done", session_id=sid, message_id=message_id)
        except (OSError, ValueError):
            pass
        finally:
            with self.server.lock:
                if parts:
                    self.server.history[sid].append({"role": "assistant", "content": "".join(parts)})
                self.server.sessions[sid]["status"] = "idle"
            try:
                self.event("turn_done", session_id=sid)
            except (OSError, ValueError):
                pass


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
