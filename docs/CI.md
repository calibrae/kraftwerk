# CI setup notes

Two workflows live under `.github/workflows/`:

- **`ci.yml`** runs on every push/PR — Rust unit tests on Linux +
  macOS + Windows, a frontend lint pass, and (main-only) integration
  tests against the live testhost hypervisor via a self-hosted runner.
- **`release.yml`** runs on `v*` tags and on manual dispatch — builds
  signed bundles for each OS and publishes a GitHub release with the
  artifacts attached.

## Self-hosted runner on speedwagon

The `integration-testhost` and `build-macos` jobs require a runner with
labels `self-hosted, macOS, speedwagon`.

Register (one-time):

```bash
cd ~
mkdir actions-runner-kraftwerk && cd actions-runner-kraftwerk
# Grab https://github.com/actions/runner/releases → osx-arm64 tarball
./config.sh \
  --url https://github.com/calibrae/kraftwerk \
  --token <REPO_RUNNER_TOKEN> \
  --name speedwagon-kraftwerk \
  --labels self-hosted,macOS,speedwagon \
  --unattended
./svc.sh install && ./svc.sh start
```

Get the token from
<https://github.com/calibrae/kraftwerk/settings/actions/runners/new>.

## Signing + notarization

The Developer ID Application certificate is already installed in
speedwagon's login keychain (identity 3 from `security find-identity
-v -p codesigning`). Tauri finds it automatically via the
`APPLE_SIGNING_IDENTITY` env var baked into the workflow.

Notarization uses an App Store Connect API key (the modern method —
no Apple ID password, no 2FA interruptions):

- **API key file**: `~/.privatekeys/AuthKey_Z43Q26MB7Y.p8` on
  speedwagon — already there.
- **Key ID**: `Z43Q26MB7Y` — baked into the workflow.
- **Team ID**: `XJQQCN392F` — baked into the workflow.
- **Issuer ID**: the one repo secret you need to set.

### The one secret

```bash
gh secret set APPLE_API_ISSUER --repo calibrae/kraftwerk --body '<issuer-uuid>'
```

The issuer UUID is on the same page where you generated the API key:
<https://appstoreconnect.apple.com/access/api>. It's already set as
a secret on `calibrae/virtmanager` (the old repo) — if you can't
remember the value, regenerate or look it up there.

## Windows

libvirt has no first-class Windows client. The Windows jobs are
allowed to fail (`continue-on-error: true`). They exist to notice if
someone fixes the situation upstream. Don't block releases on them.

## Triggering a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

Tag push → CI builds + signs + notarizes → artifacts uploaded to a
new GitHub release. Use the "Run workflow" button on the release
workflow page to build without tagging (artifacts only, no release).
