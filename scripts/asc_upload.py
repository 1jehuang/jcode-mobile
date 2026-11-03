#!/usr/bin/env python3
"""Upload a distribution-signed .ipa to App Store Connect (TestFlight) from
Linux via the documented Build Uploads API. No Transporter, no Mac.

Usage: asc_upload.py path/to/App.ipa
Finds the app by the .ipa's bundle id; version/build come from Info.plist.
"""
import hashlib, os, plistlib, sys, time, urllib.request, zipfile, json
import asc


def info_plist(ipa):
    with zipfile.ZipFile(ipa) as z:
        name = next(n for n in z.namelist() if n.count("/") == 2 and n.endswith(".app/Info.plist"))
        return plistlib.loads(z.read(name))


def must(status, data, what):
    if status >= 300:
        sys.exit(f"{what} failed ({status}): {json.dumps(data)[:1500]}")
    return data


def main(ipa):
    info = info_plist(ipa)
    bundle, version, build = info["CFBundleIdentifier"], info["CFBundleShortVersionString"], info["CFBundleVersion"]
    apps = must(*asc.call("GET", f"/v1/apps?filter[bundleId]={bundle}"), "find app")["data"]
    if not apps:
        sys.exit(f"no App Store Connect app record for {bundle}; create it in the web UI first")
    app_id = apps[0]["id"]
    print(f"app {app_id} {bundle} {version} ({build})")

    upload = must(*asc.call("POST", "/v1/buildUploads", {"data": {
        "type": "buildUploads",
        "attributes": {"cfBundleShortVersionString": version, "cfBundleVersion": build, "platform": "IOS"},
        "relationships": {"app": {"data": {"type": "apps", "id": app_id}}},
    }}), "create buildUpload")["data"]

    size = os.path.getsize(ipa)
    f = must(*asc.call("POST", "/v1/buildUploadFiles", {"data": {
        "type": "buildUploadFiles",
        "attributes": {"assetType": "ASSET", "fileName": os.path.basename(ipa), "fileSize": size, "uti": "com.apple.ipa"},
        "relationships": {"buildUpload": {"data": {"type": "buildUploads", "id": upload["id"]}}},
    }}), "reserve file")["data"]

    with open(ipa, "rb") as fh:
        blob = fh.read()
    ops = f["attributes"].get("uploadOperations") or []
    for i, op in enumerate(ops):
        part = blob[op["offset"]: op["offset"] + op["length"]]
        headers = {h["name"]: h["value"] for h in op.get("requestHeaders") or []}
        req = urllib.request.Request(op["url"], data=part, method=op.get("method", "PUT"), headers=headers)
        with urllib.request.urlopen(req) as r:
            r.read()
        print(f"  part {i + 1}/{len(ops)} ok")

    must(*asc.call("PATCH", f"/v1/buildUploadFiles/{f['id']}", {"data": {
        "type": "buildUploadFiles", "id": f["id"],
        "attributes": {"uploaded": True, "sourceFileChecksums": {"file": {"hash": hashlib.md5(blob).hexdigest(), "algorithm": "MD5"}}},
    }}), "commit file")

    for _ in range(120):
        s, d = asc.call("GET", f"/v1/buildUploads/{upload['id']}")
        state = d.get("data", {}).get("attributes", {}).get("state", {})
        print("  state:", json.dumps(state)[:400])
        st = state.get("state") if isinstance(state, dict) else state
        if st in ("COMPLETE", "FAILED"):
            return 0 if st == "COMPLETE" else 1
        time.sleep(15)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
