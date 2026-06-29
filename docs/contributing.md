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
cargo clippy --all-targets --all-features    # CI treats warnings as errors
cargo test --workspace
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

Unit tests live next to the code (e.g. `compression`, `pipeline`). Add tests for
new parsing, formulas, or step behaviour. Many end-to-end paths can be exercised
by putting stub executables on `$XPRESS_BIN_DIR` that mimic a tool's I/O.

## CI

`.github/workflows/ci.yml` runs fmt + clippy + tests on Linux and macOS for
every push and PR.

## Releasing

1. Update `CHANGELOG.md` (move `Unreleased` to the new version + date).
2. Bump `version` in the root `Cargo.toml` (`[workspace.package]`).
3. Tag and push:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

`.github/workflows/release.yml` then builds macOS (arm64/x86_64) and Linux
binaries plus a macOS `.app`, and attaches them to the GitHub Release.

## Licensing

xpress is MIT-licensed and contains no Clop source code (see
[`../NOTICE.md`](../NOTICE.md)). Keep new code original; when adding a bundled
tool, note its upstream licence in `NOTICE.md`.
