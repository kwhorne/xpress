# CI and repositories

Large, unoptimised images and videos creep into repositories — a 4 MB hero
image, a raw screenshot. `xpress check` catches them in pull requests.

## `xpress check`

```sh
xpress check -r --max-size 300kb --exclude node_modules public/ assets/
```

```text
❌ assets/screenshot.jpg  could be 38% smaller (156.7 KB → 97.6 KB)
❌ assets/sonoma.jpg  355.6 KB is over the 300.0 KB limit; could be 25% smaller (355.6 KB → 267.0 KB)
❌ assets/ui.png  could be 81% smaller (174.1 KB → 33.2 KB)

5 files checked, 3 with problems — `xpress optimise` would save 288.6 KB
```

It exits with status **1** when any file has a problem, and **0** when all are
fine — so it fails a CI job. **Files are never modified**: each one is
optimised into a temporary file just to measure it.

A file has a problem when:

- it's larger than `--max-size` (optional), or
- optimising would make it at least `--min-savings` percent smaller
  (default **10**).

### Why already-optimised JPEGs pass

Re-encoding any lossy image "saves" a bit by throwing away more detail — so a
naive check would flag every JPEG forever. `xpress check` instead asks whether
an image could be smaller **without a visible change**: images are measured
against a perceptual target, `--quality visually-lossless` by default. Lower it
(`--quality high`) to be stricter. Videos, audio and PDFs are measured with the
normal optimiser (`--compression` to change how hard).

### Options

| Option | Default | Meaning |
|--------|---------|---------|
| `-r, --recursive` | off | Look inside folders |
| `--max-size <size>` | none | Fail for files larger than this (`500kb`, `2mb`) |
| `--min-savings <pct>` | `10` | Fail when optimising would save at least this |
| `--quality <target>` | `visually-lossless` | How good an optimised image must still look |
| `--compression <5..100>` | config default | For video, audio and PDF |
| `--exclude <name>` | none | Skip directories with this name (repeat for more) |
| `--kind <kind>` | all | Only `image`, `video`, `pdf` or `audio` |
| `--json` | off | Machine-readable report |
| `-q, --quiet` | off | Only problems and the summary |
| `-j, --jobs <n>` | CPU count | Files in parallel |

Fix what it finds with `xpress optimise` (and commit the result).

## GitHub Action

```yaml
# .github/workflows/media.yml
name: Media
on: pull_request
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: kwhorne/xpress@v0.5.2
        with:
          paths: public assets
          max-size: 500kb
```

The action downloads the xpress release for the runner and runs
`xpress check --recursive`. Inputs:

| Input | Default | Meaning |
|-------|---------|---------|
| `paths` | `.` | Files or folders, space separated |
| `max-size` | — | Size limit per file |
| `min-savings` | `10` | Percent that counts as "unoptimised" |
| `quality` | `visually-lossless` | Perceptual target for images |
| `exclude` | `node_modules .git` | Directory names to skip |
| `version` | `latest` | xpress release, e.g. `v0.5.2` |

It runs on `ubuntu-latest` and `macos-latest` (Apple silicon). Images and PDFs
need nothing else; checking **videos or audio** needs ffmpeg on the runner
(e.g. a step `sudo apt-get install -y ffmpeg`).

Pin `version` (or the action ref) to a release so a new xpress version can't
change your CI's results unexpectedly.
