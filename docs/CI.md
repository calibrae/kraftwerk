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

Register a new runner (one-time):

```bash
cd ~
mkdir actions-runner-kraftwerk && cd actions-runner-kraftwerk
# Fetch latest osx-arm64 release from https://github.com/actions/runner/releases
# Then:
./config.sh \
  --url https://github.com/calibrae/kraftwerk \
  --token <REPO_RUNNER_TOKEN> \
  --name speedwagon-kraftwerk \
  --labels self-hosted,macOS,speedwagon \
  --unattended
# Install as a service so it survives reboots:
./svc.sh install
./svc.sh start
```

Get the token from
<https://github.com/calibrae/kraftwerk/settings/actions/runners/new>.

## Required secrets for signed releases

Set these in
<https://github.com/calibrae/kraftwerk/settings/secrets/actions>:

| Secret | Value |
|--------|-------|
| `APPLE_ID` | Your Apple ID email |
| `APPLE_APP_PASSWORD` | App-specific password generated at appleid.apple.com → Sign-in security |
| `APPLE_TEAM_ID` | `XJQQCN392F` |

The Developer ID Application certificate is already installed in
speedwagon's login keychain (identity 3 from `security find-identity
-v -p codesigning`). Tauri finds it automatically via the
`APPLE_SIGNING_IDENTITY` env var set in the workflow.

## What about Windows?

libvirt has no first-class Windows client. The `build-windows` and
the Windows leg of `ci.yml` are allowed to fail (`continue-on-error:
true`) — they exist to notice if someone fixes the situation upstream
or contributes a libvirt shim. Don't block releases on them.

## Triggering a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

Or use the "Run workflow" button on the release workflow page to
build without tagging. Both paths upload artifacts; only the tag
path creates a GitHub release.
