# Configuration

xpress works without any configuration. This page lists what you *can*
change and where xpress keeps its files.

## Defaults: `config.json`

The command line reads its defaults from a JSON file. Command-line flags always
win over it.

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Application Support/xpress/config.json` |
| Linux | `$XDG_CONFIG_HOME/xpress/config.json` (usually `~/.config/xpress/config.json`) |

`xpress config` shows the path and the values in effect. The file doesn't exist
until you create it; every field is optional:

```json
{
  "compression": 30,
  "aggressive": false,
  "backup": true,
  "strip_metadata": false,
  "preserve_dates": true
}
```

| Field | Default | Meaning | Overridden by |
|-------|---------|---------|---------------|
| `compression` | `30` | The [compression dial](optimising.md#the-compression-dial), 5–100 | `--compression` |
| `aggressive` | `false` | Use the aggressive preset (64) | `--aggressive` |
| `backup` | `true` | Keep `.<name>.orig` backups | `--no-backup` |
| `strip_metadata` | `false` | Remove EXIF/XMP | `--strip-metadata` |
| `preserve_dates` | `true` | Keep the original modification date | `--no-preserve-dates` |

> The desktop app has its own settings in **Preferences**; they currently last
> until you quit the app and don't read `config.json`.

## Saved pipelines and automations: `pipelines.json`

Pipelines saved with `xpress pipeline add` and folder automations made with
`xpress pipeline attach` live in `pipelines.json` next to `config.json`. Manage
them with the `pipeline` commands rather than editing by hand — see
[Pipelines](pipelines.md) and [Watching](daemon.md).

## Environment variables

| Variable | Effect |
|----------|--------|
| `XPRESS_BIN_DIR` | A folder to look in first for external tools (ffmpeg, gifsicle, …). Point it at `/Applications/xpress.app/Contents/Resources/bin` to let the command line use the app's ffmpeg. |
| `XDG_CONFIG_HOME`, `XDG_DATA_HOME` | Linux: where config and tools folders live. |

## Where xpress puts things

| What | Where |
|------|-------|
| Backups of originals | Next to each file, as hidden `.<name>.orig` |
| Converted files | Next to the original, with the new extension |
| Optimised clipboard images | `~/Pictures/xpress` |
| `xpress web` output | `<name>-web/` next to the image (or `-o`) |
| External tools (optional) | macOS `~/Library/Application Support/xpress/bin`, Linux `~/.local/share/xpress/bin` |
| "Already optimised" marks | An extended attribute on each file (`com.xpress.optimised`; Linux `user.xpress.optimised`) — see `xattr -l <file>` |

### How external tools are found

For each tool (ffmpeg, gifsicle, gifski, gs, exiftool, heif-enc, cjxl), xpress
looks in order:

1. `$XPRESS_BIN_DIR`
2. a `bin/` folder next to the `xpress` executable (and, inside the app,
   `xpress.app/Contents/Resources/bin`)
3. the per-user tools folder above
4. your `PATH`

`xpress doctor` reports what was found.
