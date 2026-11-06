# Cloud machine and GitHub access

Open the session drawer, tap the account row, and find **Cloud machine & saved files** in Settings.

## Inspect saved state

Tap **Refresh cloud machine**. This can wake your account's cloud machine. The app uses authenticated, host-key-pinned SSH to run a fixed read-only inspection with a bounded runtime and output size.

The panel shows:

- The cloud account's home directory and up to 64 top-level entry names, including hidden names.
- Whether GitHub CLI is installed and whether its current authentication check succeeds.
- Whether a GitHub CLI configuration file exists in the configured location.
- A count of possible SSH private-key files using conventional filenames. This does not inspect key contents or establish that GitHub accepts them.
- Whether a conventional Git plaintext credential file exists.

This is an inventory, not a file-content browser or credential manager. It never displays credential contents, SSH private keys, authentication diagnostics, or secret environment values. It does not install software, log into GitHub, change Git configuration, or start an agent session. The remote inspection requires Python 3. If the refresh fails, check the connection and try again.

A successful GitHub check means access is available **now**. It can come from an environment token or saved credentials. It does not prove that credentials are stored durably. File presence likewise does not prove validity, encryption, a backup, or persistence across machine replacement.

## Set up persistent GitHub access

Tap **Draft GitHub setup request** to put a safety-focused request into the current chat composer. Existing draft text is preserved. Review and send it yourself. This is an assisted setup request, not a one-tap GitHub OAuth integration.

The request asks the agent to:

1. Inspect the cloud home mount and establish its restart/replacement persistence policy before promising durability.
2. Check GitHub CLI and existing authentication without disclosing secrets.
3. Prefer browser/device authorization that you approve yourself.
4. Explain credential storage and obtain approval before using plaintext storage.
5. After authorization, configure Git to use the GitHub CLI credential helper and verify access without printing secrets.
6. Explain how to revoke access and what survives a restart.

GitHub CLI normally uses an operating-system credential store. If unavailable, it can fall back to a plaintext configuration file. Do not assume a headless cloud machine has a keyring. See the official [GitHub CLI authentication guide](https://docs.github.com/en/github-cli/github-cli/authenticating-with-github-cli) and [`gh auth login` reference](https://cli.github.com/manual/gh_auth_login).

Do not paste tokens or private keys into chat. Do not copy your laptop's credential files to an unverified cloud location. Prefer narrowly scoped access and revoke credentials when retiring a machine.

## Persistence boundary

Your phone's Jcode account authenticates the app to Jcode. It is not a GitHub login. GitHub authentication on your laptop also does not automatically authenticate a separate cloud machine.

The app currently reports observed files and status. It does **not** verify the cloud provider's storage-retention guarantees or back up credentials. Confirm the backing mount and lifecycle policy before relying on saved credentials across machine replacement.
