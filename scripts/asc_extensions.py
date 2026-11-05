#!/usr/bin/env python3
"""Register the iOS extension bundle IDs and App Store provisioning profiles
(idempotent) and save the profiles under ~/.appstoreconnect/signing/:

  sh.jcode.mobile.rust.live   -> live.mobileprovision   (Live Activity widget)
  sh.jcode.mobile.rust.share  -> share.mobileprovision  (share extension)

Uses the distribution certificate in ~/.appstoreconnect/signing/dist.id.
Usage: scripts/asc_extensions.py
"""
import base64, os, sys

sys.path.insert(0, os.path.dirname(__file__))
from asc import call

SIGN = os.path.expanduser("~/.appstoreconnect/signing")
EXT = {"live": "jcode rs Live Activity", "share": "jcode rs Share"}
APP = "sh.jcode.mobile.rust"


def bundle_id(identifier, name):
    s, d = call("GET", f"/v1/bundleIds?filter[identifier]={identifier}")
    for x in d.get("data", []):
        if x["attributes"]["identifier"] == identifier:
            return x["id"]
    s, d = call("POST", "/v1/bundleIds", {"data": {"type": "bundleIds", "attributes": {
        "identifier": identifier, "name": name, "platform": "IOS"}}})
    if s != 201:
        sys.exit(f"bundleId {identifier}: {s} {d}")
    print(f"registered {identifier}")
    return d["data"]["id"]


def profile(bid, name, cert):
    s, d = call("GET", f"/v1/bundleIds/{bid}/profiles")
    for x in d.get("data", []):
        a = x["attributes"]
        if a["profileType"] == "IOS_APP_STORE" and a["profileState"] == "ACTIVE":
            return a["profileContent"]
    s, d = call("POST", "/v1/profiles", {"data": {"type": "profiles",
        "attributes": {"name": name, "profileType": "IOS_APP_STORE"},
        "relationships": {
            "bundleId": {"data": {"type": "bundleIds", "id": bid}},
            "certificates": {"data": [{"type": "certificates", "id": cert}]}}}})
    if s != 201:
        sys.exit(f"profile {name}: {s} {d}")
    print(f"created profile {name}")
    return d["data"]["attributes"]["profileContent"]


def main():
    cert = open(f"{SIGN}/dist.id").read().strip()
    for suffix, name in EXT.items():
        bid = bundle_id(f"{APP}.{suffix}", name)
        content = profile(bid, f"{name} AppStore", cert)
        path = f"{SIGN}/{suffix}.mobileprovision"
        with open(path, "wb") as f:
            f.write(base64.b64decode(content))
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
