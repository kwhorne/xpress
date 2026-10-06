# Contributing

## Development workflow

```sh
cargo build --workspace
cargo test --workspace
cargo run -p xpress-cli -- doctor
cargo run -p xpress-gui --release
```

Before pushing, run the same checks CI does:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo audit                                  # CI also checks advisories
```

`cargo fmt --all` applies formatting; `cargo clippy --fix` can auto-apply many
lint fixes.

## Project layout

See [architecture](architecture.md). The engine lives in `xpress-core`; keep
logic there (and unit-tested) and keep the binaries thin.

## Features

- `embed-tools` (`xpress-cli`, `xpress-core`): embed vendored binaries into the
  executable. Populate `vendor/bin/<target>/` and link `current/` first
  (`scripts/fetch-tools.sh --vendor`).
- `clipboard` (`xpress-cli`, on by default): clipboard watching via `arboard`.

## Tests

- **Unit tests** live next to the code (`compression`, `pipeline`, `cache`,
  `privacy`, `video` banner parsing, …).
- **`crates/xpress-core/tests/`**: `images.rs`, `pdf.rs`, `quality.rs`,
  `web.rs` and `cache.rs` use the real in-process codecs; `integration.rs`
  installs **stub tools** (a fake `ffmpeg` that halves files, logs its
  arguments next to the input and reports a 10 s 1080p clip — or HDR for
  `*hdr*` names, a bigger output for `*grow*` names) to test video, audio and
  placement logic without real encodes.
- **GUI**: `egui_kittest` drives the app headlessly (`crates/xpress-gui`,
  `app::tests`) — navigation, crop, conversion menus, history.
- **Sync**: `sync::tests` simulates several Macs sharing one folder.
- **Apple Intelligence**: `tests/intelligence.rs` runs the real on-device
  model when it's available — build the helper first with
  `scripts/build-xpress-ai.sh` (needs Xcode/Command Line Tools 26+); it skips
  otherwise.

Test real behaviour with real files when changing codecs: e.g. compare sizes at
equal SSIMULACRA2 (`xpress_core::quality::score`) before and after.

## CI

`.github/workflows/ci.yml` runs fmt + clippy, tests on Linux and macOS, a smoke
test against real tools, and `cargo audit`, for every push to `main` and every
pull request.

## Releasing

1. Update `CHANGELOG.md` (move `Unreleased` to the new version + date, and add
   the compare link at the bottom).
2. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and commit
   `Release X.Y.Z`.
3. Tag (annotated) and push:

   ```sh
   git tag -a vX.Y.Z -m "xpress X.Y.Z"
   git push origin main vX.Y.Z
   ```

The [Homebrew tap](https://github.com/kwhorne/homebrew-tap) picks the release up
within six hours (or run its *Update* workflow by hand) and tests the new formula
and cask.

`.github/workflows/release.yml` then builds macOS (Apple silicon and Intel) and Linux
binaries, signs and notarises the macOS binaries, `.app` and `.dmg` (see
[Code signing](signing.md)), and attaches everything to the GitHub Release.

Keep the asset names as they are: `xpress update` picks the **first** asset
(alphabetically) containing the target triple, which must be the CLI tarball —
hence `xpress-<tag>-macos-<target>-app.zip` for the app (a test in
`update.rs` guards this).

## Licensing

xpress is MIT-licensed and contains no Clop source code (see
[`../NOTICE.md`](../NOTICE.md)). Keep new code original; when adding a bundled
tool, note its upstream licence in `NOTICE.md`.
