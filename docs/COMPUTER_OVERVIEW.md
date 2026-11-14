# Start-screen cloud computer overview

Empty conversations show a quiet computer card above the session and directory suggestions, including when there are no suggestions yet. The title prefers the account's Google `given_name`, then the first word of its full `name`, or “Your Jcode computer” when neither is available. Email addresses and email-derived usernames are never used as names.

After a session connects, one background inspection per signed-in app lifecycle fetches a snapshot over the existing pinned SSH inspection path. It does not block chat startup, send a prompt, or produce haptic feedback. Signing out clears the snapshot and request state. Settings → Refresh cloud machine updates the same snapshot and permits retry after a failed background request.

The card shows logical CPU count, available/total RAM, and free/total disk space for the cloud home filesystem, in GiB. These are point-in-time observations, not a live utilization monitor or the phone's hardware. Unsupported or unavailable readings show an explicit unavailable state rather than invented values. No credentials, file contents, or host authentication details appear in the card.

The fixed read-only inspection collects CPU count with `os.cpu_count()`, Linux memory counters from `/proc/meminfo`, and home filesystem capacity with `shutil.disk_usage(home)`. Resource fields are bounded and validated before rendering. Older snapshots without resource fields remain accepted.

Google sign-in already requests the `profile` scope. The backend stores optional `given_name` and `name` claims after validating the Google identity and returns them in login and authenticated profile responses. The SDK persists these optional fields, accepting older account JSON without them. Once per signed-in app lifecycle, mobile silently refreshes the saved profile through `/auth/verify` without changing credentials, email, chat sessions, or drafts. Stale results from a previous login are ignored. Connection and inspection guards use credentials rather than display metadata so a refreshed name cannot invalidate work in flight.

An account created before name storage was enabled needs one new Google sign-in to populate its name. Names cannot be recovered from an email address, and no Google token is retained for later profile retrieval. Backend rollout requires the additive account-profile migration before deploying the corresponding API changes.
