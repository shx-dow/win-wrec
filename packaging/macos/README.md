# macOS packaging

`scripts/package-macos.sh` assembles the macOS app bundle:

```text
Wrec*.app/
  Contents/
    Info.plist
    MacOS/
      wrec-app
      daemon
      capture-engine
    Resources/
```

The packaged app resolves `daemon` beside its executable. The daemon resolves
`capture-engine` beside its executable at runtime. Cargo development still falls
back to the capture-engine path emitted by `crates/macos/build.rs`.

For contributor/dev packaging:

```bash
./scripts/package-macos.sh
```

This creates `dist/dev/Wrec Dev.app` with the dev Cargo profile, ad-hoc
signing, bundle id `app.wrec.wrec.dev`, app data in
`~/Library/Application Support/Wrec Dev`, and recordings in `~/Movies/Wrec Dev`.
It also writes `dist/dev/README.md` on every run with the local commands and
build details for that generated app.

Dev packaging uses `images/wrec-dev.png` as the app icon.

For release packaging:

```bash
./scripts/package-macos.sh release
```

This creates `dist/release/Wrec.app` with the release Cargo profile,
bundle id `app.wrec.wrec`, and a `.dmg` by default. Release packaging does not
generate the companion README.

Release packaging uses `images/wrec.png` as the app icon.

For Developer ID signing a release:

```bash
CODESIGN_IDENTITY="Developer ID Application: Example, Inc. (TEAMID)" \
./scripts/package-macos.sh release
```

For notarization, provide App Store Connect credentials and enable notarization:

```bash
CODESIGN_IDENTITY="Developer ID Application: Example, Inc. (TEAMID)" \
APPLE_ID="dev@example.com" \
APPLE_TEAM_ID="TEAMID" \
APPLE_APP_PASSWORD="app-specific-password" \
NOTARIZE=1 \
./scripts/package-macos.sh release
```

Set `ICON_SOURCE=/path/to/icon.png` to override the channel's default icon.

## CLI packaging

`scripts/package-cli-macos.sh` assembles the standalone CLI runtime:

```text
wrec-cli/
  wrec
  daemon
  capture-engine
```

The resulting archive is written to `dist/cli/wrec-cli-<target>.tar.gz`.
`scripts/install-cli.sh` installs that runtime under `/usr/local/lib/wrec` and
places a managed wrapper at `/usr/local/bin/wrec`.

This package is intentionally separate from the app bundle. It carries the same
daemon and capture-engine runtime so terminal users and agents can install
`wrec` without copying files out of `Wrec.app`.

## GitHub release workflow

`.github/workflows/release.yml` publishes macOS release downloads when a `v*`
tag is pushed and the tagged commit is on `origin/main`. GitHub Actions cannot
filter tags by source branch in the trigger itself, so the workflow does an
explicit ancestry check before packaging.

The workflow uploads the notarized `.dmg` and the standalone CLI runtime archive
as GitHub Release assets. Required repository secrets:

- `MACOS_CERTIFICATE_BASE64` - base64-encoded Developer ID Application `.p12`
- `MACOS_CERTIFICATE_PASSWORD` - password for that `.p12`
- `MACOS_KEYCHAIN_PASSWORD` - temporary CI keychain password
- `MACOS_CODESIGN_IDENTITY` - Developer ID Application identity name
- `APPLE_ID` - App Store Connect Apple ID
- `APPLE_TEAM_ID` - Apple developer team id
- `APPLE_APP_PASSWORD` - app-specific password for notarization
