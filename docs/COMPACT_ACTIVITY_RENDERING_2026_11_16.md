# Compact activity rendering, November 16, 2026

## Behavior

Background task updates, swarm messages and agent reports, file activity, notifications, and reserved restart messages use shared native activity disclosures in normal and synced transcripts. Collapsed rows contain an appropriate icon, concise summary, approximate `~N tok` badge, and disclosure indicator. Token counts estimate text size, not billed provider usage. Expanding preserves the full Markdown detail. Machine updates do not increment human prompt numbering.

Live session notices use the same renderer, including notice-only sessions and parked previews. Images remain visible independently of disclosure state. Failed thumbnail decoding now shows an explicit unavailable state instead of an indefinitely blank tile. Expansion invalidates virtualized transcript geometry without sharing disclosure state between unrelated rows.

## Validation

- Hosted mobile check, workspace library tests, Clippy, and release build passed on the final source: 699 passed, 0 failed, 16 ignored.
- Desktop producer mobile-sync tests: 35 passed, 0 failed, 4 ignored.
- Backend mobile-sync contract tests: 24 passed, 0 failed.
- Native Linux GPUI preview verified compact rows, visible thumbnails, and expandable details using a fake account and GET-only local fixture. Narrow light-mode acceptance includes 320px width at 1.6x text scale. This is not physical iPhone or Apple Simulator acceptance.

## Coordinated source and rollout boundary

Desktop commits `baed38b29` and `2142e70d9` retain visible user/tool image blocks and image-only messages, generate bounded JPEG thumbnails, and enable GIF/WebP decoding. Hidden provider visual-context images remain excluded. Eligible historical replay retains its existing 14-day window.

Backend commit `eb5a7d3` on the existing `feat/browser-code-subscribe` branch accepts validated attachments without changing their original transcript roles. The existing image validation limits and account isolation remain enforced.

These producer and backend changes are committed and pushed, not deployed or activated by this work. Deploy the compatible backend change before activating the new producer. Do not deploy the unrelated feature branch wholesale. These changes are not included in the previously published TestFlight build 1794827452.

## TestFlight release

The mobile rendering changes are now available in TestFlight **1.0 (1794831158)**, built from immutable mobile commit `032da214dce1096db1ff0cdc16cfc7f0b6fbd784` with unchanged voice dependency commit `beaa566b86ff3e87d3f9ce8ff5eb36f64a04637e`.

- Signed IPA SHA-256: `274ee99d905a2f1750b4cf1ebb0d2249552fa6a18a148e28d5dfcf7c0d1c96ff`.
- Apple build ID: `138864da-e281-443e-be66-d3f796905787`.
- Independently verified November 16, 2026 at 12:18:47 UTC: processing `VALID`, internal and external state `IN_BETA_TESTING`, automatic notification enabled, and membership in the public-link-enabled Public Beta group.
- All four app bundle versions and three embedded provisioning profiles were verified. Downloaded IPA hash, ZIP CRC, source provenance, and actual release-visible activity/image renderer strings were checked before upload.
- This mobile release does not deploy the backend or activate the desktop producer. The image-upload role compatibility rollout gate above still applies.
