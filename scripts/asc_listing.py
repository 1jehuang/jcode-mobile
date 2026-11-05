#!/usr/bin/env python3
"""Push appstore/metadata.json + screenshots to the App Store Connect version.

Usage: asc_listing.py [--submit]
Review sign-in code comes from ~/.appstoreconnect/review_login.txt ("email code").
Idempotent: re-running replaces screenshots and overwrites text fields.
"""
import hashlib, json, os, sys, urllib.request
sys.path.insert(0, os.path.dirname(__file__))
from asc import call

APP = "6830033634"
ROOT = os.path.join(os.path.dirname(__file__), "..", "appstore")


def ok(status, data, what):
    if status >= 300:
        sys.exit(f"{what}: HTTP {status} {json.dumps(data)[:800]}")
    return data


def get(path):
    return ok(*call("GET", path), path)


def patch(type_, id_, attrs, what):
    ok(*call("PATCH", f"/v1/{type_}/{id_}", {"data": {"type": type_, "id": id_, "attributes": attrs}}), what)


def main():
    meta = json.load(open(os.path.join(ROOT, "metadata.json")))
    version = next(v for v in get(f"/v1/apps/{APP}/appStoreVersions")["data"]
                   if v["attributes"]["appStoreState"] in ("PREPARE_FOR_SUBMISSION", "DEVELOPER_REJECTED", "REJECTED"))
    vid = version["id"]
    print("version", version["attributes"]["versionString"], vid)

    patch("appStoreVersions", vid, {"copyright": meta["copyright"]}, "copyright")
    loc = get(f"/v1/appStoreVersions/{vid}/appStoreVersionLocalizations")["data"][0]
    patch("appStoreVersionLocalizations", loc["id"], {k: meta[k] for k in
          ("description", "keywords", "promotionalText", "supportUrl", "marketingUrl")}, "version localization")

    info = next(i for i in get(f"/v1/apps/{APP}/appInfos")["data"]
                if i["attributes"]["state"] != "READY_FOR_DISTRIBUTION")
    iloc = get(f"/v1/appInfos/{info['id']}/appInfoLocalizations")["data"][0]
    patch("appInfoLocalizations", iloc["id"], {"name": meta["name"], "subtitle": meta["subtitle"],
          "privacyPolicyUrl": meta["privacyPolicyUrl"]}, "app info localization")
    ok(*call("PATCH", f"/v1/appInfos/{info['id']}", {"data": {"type": "appInfos", "id": info["id"], "relationships": {
        "primaryCategory": {"data": {"type": "appCategories", "id": meta["primaryCategory"]}}}}}), "category")

    # Age rating: a developer tool with no objectionable content. Booleans are
    # the yes/no questions, the rest are NONE/INFREQUENT_OR_MILD/... levels.
    decl = get(f"/v1/appInfos/{info['id']}/ageRatingDeclaration")["data"]
    booleans = {"advertising", "ageAssurance", "gambling", "healthOrWellnessTopics", "lootBox",
                "messagingAndChat", "parentalControls", "socialMedia", "socialMediaAgeRestricted",
                "unrestrictedWebAccess", "userGeneratedContent"}
    levels = {"alcoholTobaccoOrDrugUseOrReferences", "contests", "gamblingSimulated", "gunsOrOtherWeapons",
              "medicalOrTreatmentInformation", "profanityOrCrudeHumor", "sexualContentGraphicAndNudity",
              "sexualContentOrNudity", "horrorOrFearThemes", "matureOrSuggestiveThemes",
              "violenceCartoonOrFantasy", "violenceRealisticProlongedGraphicOrSadistic", "violenceRealistic"}
    attrs = {k: False for k in booleans if k in decl["attributes"]}
    attrs.update({k: "NONE" for k in levels if k in decl["attributes"]})
    status, data = call("PATCH", f"/v1/ageRatingDeclarations/{decl['id']}",
                        {"data": {"type": "ageRatingDeclarations", "id": decl["id"], "attributes": attrs}})
    if status >= 300:
        print("age rating needs manual answers:", json.dumps(data)[:600])

    # Review details with the demo sign-in.
    email, code = open(os.path.expanduser("~/.appstoreconnect/review_login.txt")).read().split()
    r = meta["review"]
    review = {"demoAccountRequired": True, "demoAccountName": email, "demoAccountPassword": code,
              "notes": r["notes"], "contactFirstName": r["contactFirstName"], "contactLastName": r["contactLastName"],
              "contactEmail": r["contactEmail"], "contactPhone": r["contactPhone"]}
    review = {k: v for k, v in review.items() if v != ""}
    status, existing = call("GET", f"/v1/appStoreVersions/{vid}/appStoreReviewDetail")
    if status == 200 and existing.get("data"):
        s, d = call("PATCH", f"/v1/appStoreReviewDetails/{existing['data']['id']}", {"data": {
            "type": "appStoreReviewDetails", "id": existing["data"]["id"], "attributes": review}})
    else:
        s, d = call("POST", "/v1/appStoreReviewDetails", {"data": {"type": "appStoreReviewDetails", "attributes": review,
           "relationships": {"appStoreVersion": {"data": {"type": "appStoreVersions", "id": vid}}}}})
    if s >= 300:
        print("review details NOT saved (set contactPhone in metadata.json):", json.dumps(d)[:300])

    upload_screenshots(loc["id"])

    build = get(f"/v1/builds?filter[app]={APP}&filter[processingState]=VALID&sort=-uploadedDate&limit=1")["data"][0]
    ok(*call("PATCH", f"/v1/appStoreVersions/{vid}/relationships/build",
             {"data": {"type": "builds", "id": build["id"]}}), "attach build")
    print("build", build["attributes"]["version"])
    if build["attributes"].get("usesNonExemptEncryption") is None:
        patch("builds", build["id"], {"usesNonExemptEncryption": False}, "export compliance")

    if "--submit" in sys.argv:
        submit(vid)


def upload_screenshots(loc_id):
    sets = get(f"/v1/appStoreVersionLocalizations/{loc_id}/appScreenshotSets")["data"]
    sset = next((s for s in sets if s["attributes"]["screenshotDisplayType"] == "APP_IPHONE_67"), None)
    if sset is None:
        sset = ok(*call("POST", "/v1/appScreenshotSets", {"data": {"type": "appScreenshotSets",
                  "attributes": {"screenshotDisplayType": "APP_IPHONE_67"}, "relationships": {
                  "appStoreVersionLocalization": {"data": {"type": "appStoreVersionLocalizations", "id": loc_id}}}}}),
                  "screenshot set")["data"]
    existing = get(f"/v1/appScreenshotSets/{sset['id']}/appScreenshots")["data"]
    folder = os.path.join(ROOT, "screenshots", "6.9")
    local = {n: hashlib.md5(open(os.path.join(folder, n), "rb").read()).hexdigest() for n in sorted(os.listdir(folder))}
    remote = {s["attributes"]["fileName"]: s["attributes"].get("sourceFileChecksum") for s in existing}
    if remote == local:
        return  # unchanged: re-uploading would restart Apple's processing
    for old in existing:
        call("DELETE", f"/v1/appScreenshots/{old['id']}")
    for name in sorted(os.listdir(folder)):
        blob = open(os.path.join(folder, name), "rb").read()
        shot = ok(*call("POST", "/v1/appScreenshots", {"data": {"type": "appScreenshots",
                  "attributes": {"fileName": name, "fileSize": len(blob)}, "relationships": {
                  "appScreenshotSet": {"data": {"type": "appScreenshotSets", "id": sset["id"]}}}}}), name)["data"]
        for op in shot["attributes"]["uploadOperations"]:
            part = blob[op["offset"]:op["offset"] + op["length"]]
            req = urllib.request.Request(op["url"], data=part, method=op["method"],
                                         headers={h["name"]: h["value"] for h in op["requestHeaders"]})
            urllib.request.urlopen(req).read()
        patch("appScreenshots", shot["id"], {"uploaded": True,
              "sourceFileChecksum": hashlib.md5(blob).hexdigest()}, f"commit {name}")
        print("screenshot", name)


def submit(vid):
    sub = ok(*call("POST", "/v1/reviewSubmissions", {"data": {"type": "reviewSubmissions",
             "attributes": {"platform": "IOS"}, "relationships": {"app": {"data": {"type": "apps", "id": APP}}}}}),
             "review submission")["data"]
    ok(*call("POST", "/v1/reviewSubmissionItems", {"data": {"type": "reviewSubmissionItems", "relationships": {
        "reviewSubmission": {"data": {"type": "reviewSubmissions", "id": sub["id"]}},
        "appStoreVersion": {"data": {"type": "appStoreVersions", "id": vid}}}}}), "submission item")
    patch("reviewSubmissions", sub["id"], {"submitted": True}, "submit")
    print("submitted for review", sub["id"])


if __name__ == "__main__":
    main()
