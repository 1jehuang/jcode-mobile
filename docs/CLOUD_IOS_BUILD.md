# Managed-cloud iOS release preparation

## Target and trust boundary

Use the account-managed `jcode-cloud`, not a similarly named personal VM:

```sh
jcode cloud machine status
ssh -F "$HOME/.jcode/cloud/ssh_config" jcode-cloud 'hostname; id'
```

On 2026-11-06 the verified machine was `ip-172-31-1-212.ec2.internal`,
Amazon Linux 2023 x86_64, user `ec2-user`. The repository is
`/home/jeremy/jcode-mobile`. Generated private SSH configuration supplies the
managed proxy and host-key checking. Plain `ssh jcode-cloud` need not resolve.

The home directory resides on root XFS `/dev/nvme0n1p1`. The managed CLI documents
that the disk persists while sleeping. This is not a backup or proof of retention
after machine deletion/replacement. Private signing and API keys remain on that
disk until removed or revoked. Only explicitly authorized credentials should be
provisioned, never a complete home directory, keychain or SSH private key.

## Git access

From the local checkout, use the supported no-clobber credential mechanism:

```sh
jcode cloud env push --host jcode-cloud --request '{"action":"fetch","resource":{"kind":"credential","name":"github"}}'
```

The remote repository must already exist at the same absolute path. Existing
credentials are skipped, not replaced. Install a Linux-compatible GitHub CLI and
run `gh auth setup-git` on the VM. Verify with `git ls-remote origin HEAD`,
`git push --dry-run origin HEAD:refs/heads/main`, and the repository API's
`permissions.push`. These checks do not publish commits or create branches.
Never print `gh auth token` or credential file contents.

## Toolchain

The prepared machine has Rust `1.98.1` with `aarch64-apple-ios`, Swift `6.4.1`,
and the existing Darwin SDK artifact bundle copied from the working release
machine. Preserve symlinks and hardlinks and use compression (`rsync -aHz`) for
these large, nonsecret directories:

- `~/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu`
- `~/.local/share/swiftly/toolchains/6.4.1`
- `~/.swiftpm/swift-sdks/darwin.artifactbundle`

Apple SDK access remains subject to the applicable Apple license. Do not publish
the SDK or put it in Git. Amazon Linux's glibc 2.34 cannot execute this Swift and
rcodesign distribution. `scripts/cloud-ios.Dockerfile` uses Ubuntu 24.04 instead
of replacing system libc. Docker is installed on the host. No credentials enter
the image or build context.

The isolated `~/.local/share/jcode-mobile-build/bin` contains `justrust`,
`rcodesign`, `ios-clang`, and `rustup`, with `cargo`, `rustc`, and `rustdoc`
symlinks to `rustup`. These are tools, not credentials. Cargo cache and build
state are separate from the account's existing configuration. The runner mounts
only the repository, these tools, the SDK/toolchains, and signing profiles.

## Approved signing and API resources

Only these resources were approved for private pinned-SSH, no-clobber transfer:

- `~/.appstoreconnect/signing/dist.p12`
- `~/.appstoreconnect/signing/appstore.mobileprovision`
- `~/.appstoreconnect/signing/live.mobileprovision`
- `~/.appstoreconnect/signing/share.mobileprovision`
- `~/.appstoreconnect/AuthKey_5Z55356KJR.p8`

Directories are mode `0700`, files `0600`. Do not transfer other identities,
review logins, certificate private-key source files, or the whole account folder.
The existing release script contains the PKCS#12 password, so file permissions
and disk access, not that password, are the meaningful protection. Rotate/revoke
keys if this VM is retired or compromised. The API key is deliberately **not**
mounted into the build container. A host-side read-only `scripts/asc.py GET`
request can verify the key without sending a release.

## Build without uploading

On the VM:

```sh
cd /home/jeremy/jcode-mobile
scripts/cloud-ios-build.sh
```

The output is `target/ios/JcodeMobile.ipa`. The build includes Swift bridge,
Live Activity and share extensions, embeds the profiles, and signs the bundles.
`justrust` runs on this VM in explicit local mode because the Apple cross-build
requires the mounted SDK and linker. Two compile jobs limit memory pressure on
the 8 GB VM. Unstripped symbolication binaries are archived under
`~/.local/share/jcode-mobile-build/state/.appstoreconnect/builds/BUILD_NUMBER`.
The script uses a fresh timestamp build number unless `BUILD_NUMBER` is set.

This builds a candidate only. **It never uploads to TestFlight.** Do not upload a
cloud proof build when a local release has already been uploaded. For a future
separately authorized release, the existing `scripts/asc_upload.py` implements
the App Store Connect upload path and can run on the host with Python
`cryptography` and the specifically provisioned API key. Confirm the exact
candidate and build number before invoking it.
