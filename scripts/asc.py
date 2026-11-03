#!/usr/bin/env python3
"""Minimal App Store Connect API client (stdlib + cryptography).

Usage:
  asc.py [--key ID] GET /v1/apps
  asc.py [--key ID] POST /v1/bundleIds '{"data": ...}'
Env: ASC_KEY_ID, ASC_ISSUER_ID (defaults below), keys in ~/.appstoreconnect.
"""
import base64, json, os, sys, time, urllib.request, urllib.error
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

ISSUER = os.environ.get("ASC_ISSUER_ID", "f1147f07-48fe-4850-9171-f37d4b2dee41")
KEY_DIR = os.path.expanduser("~/.appstoreconnect")


def b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def token(key_id):
    with open(f"{KEY_DIR}/AuthKey_{key_id}.p8", "rb") as f:
        key = serialization.load_pem_private_key(f.read(), None)
    head = b64(json.dumps({"alg": "ES256", "kid": key_id, "typ": "JWT"}).encode())
    now = int(time.time())
    body = b64(json.dumps({"iss": ISSUER, "iat": now, "exp": now + 1000, "aud": "appstoreconnect-v1"}).encode())
    r, s = decode_dss_signature(key.sign(f"{head}.{body}".encode(), ec.ECDSA(hashes.SHA256())))
    return f"{head}.{body}.{b64(r.to_bytes(32, 'big') + s.to_bytes(32, 'big'))}"


def call(method, path, body=None, key_id=None):
    key_id = key_id or os.environ.get("ASC_KEY_ID", "5Z55356KJR")
    req = urllib.request.Request(
        "https://api.appstoreconnect.apple.com" + path,
        data=json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={"Authorization": f"Bearer {token(key_id)}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req) as resp:
            raw = resp.read()
            return resp.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            return e.code, json.loads(raw)
        except Exception:
            return e.code, {"raw": raw.decode(errors="replace")}


if __name__ == "__main__":
    args = sys.argv[1:]
    key = None
    if args[:1] == ["--key"]:
        key, args = args[1], args[2:]
    method, path = args[0], args[1]
    body = json.loads(args[2]) if len(args) > 2 else None
    status, data = call(method, path, body, key)
    print(status)
    print(json.dumps(data, indent=1)[:6000])
