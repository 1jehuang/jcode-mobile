# Start-screen cloud computer overview

Empty conversations show a quiet computer card above the session and directory suggestions, including when there are no suggestions yet. The title uses the signed-in email's local name, or “Your Jcode computer” when no name is available.

After a session connects, one background inspection per signed-in app lifecycle fetches a snapshot over the existing pinned SSH inspection path. It does not block chat startup, send a prompt, or produce haptic feedback. Signing out clears the snapshot and request state. Settings → Refresh cloud machine updates the same snapshot and permits retry after a failed background request.

The card shows logical CPU count, available/total RAM, and free/total disk space for the cloud home filesystem, in GiB. These are point-in-time observations, not a live utilization monitor or the phone's hardware. Unsupported or unavailable readings show an explicit unavailable state rather than invented values. No credentials, file contents, or host authentication details appear in the card.

The fixed read-only inspection collects CPU count with `os.cpu_count()`, Linux memory counters from `/proc/meminfo`, and home filesystem capacity with `shutil.disk_usage(home)`. Resource fields are bounded and validated before rendering. Older snapshots without resource fields remain accepted.
