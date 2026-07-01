# Code signing & notarisation (macOS)

For a `.app`/`.dmg` to open without warnings on other Macs it must be **signed
with a Developer ID** and **notarised** by Apple.

> xpress can't generate a Developer ID certificate — Apple issues those to your
> paid Developer account. If `security find-identity -v -p codesigning` lists a
> `Developer ID Application: …` identity, you already have one.

## Local builds

`scripts/make-app.sh` and `scripts/make-dmg.sh` automatically sign with the first
`Developer ID Application` identity in your keychain (or the one in
`$XPRESS_SIGN_ID`), using a hardened runtime. Without one they fall back to an
ad-hoc signature for local use.

```sh
cargo build --release -p xpress-gui -p xpress-cli
scripts/make-app.sh            # -> dist/xpress.app  (Developer ID signed)
scripts/make-dmg.sh            # -> dist/xpress.dmg  (signed)
```

### Notarise locally

Create an **app-specific password** at
<https://appleid.apple.com> → *Sign-In & Security*, then:

```sh
export APPLE_ID="you@example.com"
export APPLE_TEAM_ID="7G383N3VY7"
export APPLE_APP_PASSWORD="xxxx-xxxx-xxxx-xxxx"
export XPRESS_NOTARIZE=1
scripts/make-app.sh            # signs, submits to Apple, staples the ticket
scripts/make-dmg.sh
# or notarise an existing artifact directly:
scripts/notarize.sh dist/xpress.dmg
```

(Alternatively use an App Store Connect API key: `ASC_KEY_ID`, `ASC_ISSUER_ID`,
`ASC_KEY_PATH`.)

Verify:

```sh
spctl -a -vvv -t exec dist/xpress.app   # should say "accepted / Notarized Developer ID"
```

## Release workflow (CI)

`.github/workflows/release.yml` signs and notarises automatically **when the
following repository secrets are set** (Settings → Secrets and variables →
Actions). Without them the release still builds, just unsigned.

| Secret | What it is |
|--------|------------|
| `MACOS_CERT_P12` | Your Developer ID Application cert + key, exported as `.p12` and base64-encoded |
| `MACOS_CERT_PASSWORD` | Password you set when exporting the `.p12` |
| `MACOS_SIGN_IDENTITY` | e.g. `Developer ID Application: GETS AS (7G383N3VY7)` |
| `APPLE_ID` | Your Apple ID email (for notarisation) |
| `APPLE_TEAM_ID` | e.g. `7G383N3VY7` |
| `APPLE_APP_PASSWORD` | App-specific password |

### Exporting the certificate

1. Open **Keychain Access**, find *Developer ID Application: …* (with its private key).
2. Right-click → **Export** → save as `cert.p12` with a password.
3. Base64-encode it and copy to the clipboard:

   ```sh
   base64 -i cert.p12 | pbcopy
   ```

4. Paste as the `MACOS_CERT_P12` secret; set the others as above.

Tag a release (`git tag vX.Y.Z && git push origin vX.Y.Z`) and the workflow
produces a signed, notarised `.app` zip and `.dmg`.
