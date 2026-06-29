# Daemon & automations

`xpress watch` runs in the foreground and optimises files automatically as they
appear — in watched folders and/or on the clipboard. Press `Ctrl-C` to stop.

## Folder automations

Attach a pipeline to a folder, then start the watcher:

```sh
xpress pipeline attach ~/Screenshots 'crop(longEdge: 2000) -> convert(to: webp)' --type image
xpress watch                 # uses all saved automations
```

Or watch ad-hoc folders with an explicit pipeline:

```sh
xpress watch --pipeline 'optimise' ~/Inbox ~/Downloads
xpress watch -r ~/Media       # recurse into subfolders
```

How it behaves:

- New or changed files trigger their folder's pipeline.
- Events are **debounced** (~0.5s quiet period) so partially-written files settle.
- The `--type` filter (`all|image|video|audio|pdf`) limits which files run.
- **Loop prevention**: after writing a result, xpress records the file's new
  mtime and ignores the resulting change event, so in-place results don't
  re-trigger endlessly.
- Hidden files, `.orig` backups and `~` temp files are skipped.

Manage attachments:

```sh
xpress pipeline list                 # shows folder automations too
xpress pipeline detach ~/Screenshots
```

Attachments are stored in the per-user config file
(`~/Library/Application Support/xpress/pipelines.json` or the XDG/`APPDATA`
equivalent).

## Clipboard watching

```sh
xpress watch --clipboard            # clipboard only
xpress watch --clipboard ~/Inbox    # clipboard + a folder
```

When an image is copied, xpress encodes it to PNG, optimises it (or runs the
`--pipeline` if given), and saves the result to `~/Pictures/xpress`.

Requires the `clipboard` feature (on by default) and `ffmpeg` for the
raw→PNG step. Add a clipboard automation so plain `xpress watch` includes it:

```sh
xpress pipeline attach clipboard 'optimise'
```

## Running it in the background

`watch` is a foreground process. To keep it running, use your OS service manager
— e.g. a macOS LaunchAgent or a systemd user service — pointing at
`xpress watch`. (Global hotkeys for on-demand optimisation live in the
[desktop app](gui.md).)
