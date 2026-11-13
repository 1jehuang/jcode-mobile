#!/usr/bin/env python3
"""Ship a processed build to the public TestFlight group.

Usage: asc_public_beta.py [BUILD_VERSION] [--notes "What to Test text"]
Defaults to the newest VALID build. Sets en-US "What to Test", adds the build
to the "Public Beta" group and submits it for beta app review (idempotent).
"""
import json, os, sys
sys.path.insert(0, os.path.dirname(__file__))
from asc import call

APP = "6830033634"
GROUP = "Public Beta"
DEFAULT_NOTES = (
    "Jcode for iPhone: chat with your Jcode Cloud coding agent from anywhere.\n\n"
    "Please try: signing in, starting a session, sending a task and following tool calls live, "
    "switching tabs, pull-to-talk voice dictation (pull the transcript up past the bottom), "
    "and the default directory setting for new sessions.\n\n"
    "Send feedback with a screenshot from TestFlight, or email jeremyhuang55555@gmail.com."
)


def must(res, what):
    status, data = res
    if status >= 300:
        sys.exit(f"{what} failed ({status}): {json.dumps(data)[:600]}")
    return data


def main():
    args = sys.argv[1:]
    notes = DEFAULT_NOTES
    if "--notes" in args:
        i = args.index("--notes")
        notes = args[i + 1]
        del args[i:i + 2]
    q = f"/v1/builds?filter[app]={APP}&filter[processingState]=VALID&sort=-uploadedDate&limit=1"
    if args:
        q += f"&filter[version]={args[0]}"
    build = must(call("GET", q), "find build")["data"][0]
    bid, ver = build["id"], build["attributes"]["version"]
    print("build", ver, bid)

    locs = must(call("GET", f"/v1/builds/{bid}/betaBuildLocalizations"), "localizations")["data"]
    loc = next((l for l in locs if l["attributes"]["locale"] == "en-US"), None)
    if loc:
        must(call("PATCH", f"/v1/betaBuildLocalizations/{loc['id']}", {"data": {
            "type": "betaBuildLocalizations", "id": loc["id"], "attributes": {"whatsNew": notes}}}), "notes")
    else:
        must(call("POST", "/v1/betaBuildLocalizations", {"data": {"type": "betaBuildLocalizations",
             "attributes": {"locale": "en-US", "whatsNew": notes},
             "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}}), "notes")
    print("notes set")

    groups = must(call("GET", f"/v1/apps/{APP}/betaGroups"), "groups")["data"]
    group = next(g for g in groups if g["attributes"]["name"] == GROUP)
    must(call("POST", f"/v1/betaGroups/{group['id']}/relationships/builds",
              {"data": [{"type": "builds", "id": bid}]}), "add to group")
    print("added to", GROUP, group["attributes"].get("publicLink"))

    status, sub = call("GET", f"/v1/builds/{bid}/betaAppReviewSubmission")
    if status == 200 and sub.get("data"):
        print("beta review already", sub["data"]["attributes"]["betaReviewState"])
        return
    data = must(call("POST", "/v1/betaAppReviewSubmissions", {"data": {"type": "betaAppReviewSubmissions",
                "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}}), "submit beta review")
    print("beta review", data["data"]["attributes"]["betaReviewState"])


if __name__ == "__main__":
    main()
