#!/usr/bin/env python3
"""Drive a harness API bridge like the iOS app opening sessions.

usage: bridge_open_probe.py SOCKET [N_SESSIONS]
Prints client-side timings; the bridge writes server-side ones to
~/.jcode/logs/api-timing.log.
"""
import json, socket, sys, time

sock_path = sys.argv[1]
n = int(sys.argv[2]) if len(sys.argv) > 2 else 5


class Conn:
    def __init__(self):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.connect(sock_path)
        self.f = self.s.makefile("rb")
        self.id = 0

    def send(self, req, **kw):
        self.id += 1
        kw.update(v=1, id=self.id, req=req)
        self.s.sendall((json.dumps(kw) + "\n").encode())
        return self.id

    def wait(self, ids, timeout=60):
        """Read until every id in ids is answered; return {id: (ms, bytes, ev)}."""
        t0, out, ids = time.time(), {}, set(ids)
        while ids - out.keys():
            line = self.f.readline()
            if not line:
                raise SystemExit("bridge closed")
            fr = json.loads(line)
            r = fr.get("reply_to")
            if r in ids and r not in out:
                out[r] = ((time.time() - t0) * 1000, len(line), fr.get("ev"))
            if time.time() - t0 > timeout:
                break
        return out


c = Conn()
t = time.time()
c.wait([c.send("hello", min_version=1, max_version=1, client="probe")])
print(f"hello {(time.time()-t)*1000:.0f}ms")
r = c.wait([c.send("list_sessions", include_archived=False, limit=80)])
sessions = []
c2 = Conn()
c2.send("hello", min_version=1, max_version=1, client="probe")
c2.wait([1])
c2.send("list_sessions", include_archived=False, limit=80)
for line in iter(c2.f.readline, b""):
    fr = json.loads(line)
    if fr.get("reply_to") == 2:
        sessions = [s["session_id"] for s in fr.get("sessions", [])]
        break
print(f"list_sessions {list(r.values())[0][0]:.0f}ms n={len(sessions)}")

for sid in sessions[:n]:
    c = Conn()
    t = time.time()
    c.wait([c.send("hello", min_version=1, max_version=1, client="probe")])
    hello = (time.time() - t) * 1000
    t = time.time()
    a = c.wait([c.send("attach_session", session_id=sid)])
    attach = (time.time() - t) * 1000
    ids = {
        c.send("get_history", session_id=sid): "history",
        c.send("list_models", session_id=sid): "models",
        c.send("list_sessions", include_archived=False, limit=80): "sessions",
    }
    res = c.wait(ids)
    parts = " ".join(f"{ids[i]}={ms:.0f}ms/{b//1024}KB" for i, (ms, b, ev) in res.items())
    print(f"open hello={hello:.0f}ms attach={attach:.0f}ms {parts}")
    c.s.close()
