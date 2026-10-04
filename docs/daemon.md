# Watching folders and the clipboard

`xpress watch` optimises files **automatically** as they appear — in folders you
choose and/or on the clipboard. It runs until you press Ctrl-C (or as a
background service, see below).

## Watch a folder

```sh
xpress watch ~/Desktop/Screenshots                  # optimise new files there
xpress watch -r ~/Media                             # include subfolders
xpress watch --pipeline 'crop(longEdge: 2000) -> convert(to: webp)' ~/Inbox
```

For folders you always want handled, **attach** a pipeline once and start
`xpress watch` with no arguments:

```sh
xpress pipeline attach ~/Desktop/Screenshots 'convert(to: webp)' --type image
xpress pipeline attach ~/Podcasts 'normalize -> convert(to: mp3)' --type audio
xpress watch                       # runs every attachment
```

`--type` (`all`, `image`, `video`, `audio`, `pdf`) limits which files a folder's
pipeline handles. `xpress pipeline list` shows attachments;
`xpress pipeline detach <folder>` removes one.

### How it behaves

- **Waits for files to finish.** A file is processed only once it has stopped
  changing for about 1.5 seconds, so large copies, downloads and recordings
  aren't optimised half-written.
- **Doesn't loop.** Files xpress writes itself — the optimised file, or a new
  one such as `clip.mp4` or `photo.webp` — don't trigger another run.
- **Skips** hidden files, `.orig` backups and `~` temporary files.
- Uses the same safety rules as `optimise`: backups, never bigger, dates kept,
  and the [common options](cli.md#common-options) (`--aggressive`,
  `--strip-location`, `--no-backup`, …) apply.

## Watch the clipboard

```sh
xpress watch --clipboard                # clipboard only
xpress watch --clipboard ~/Inbox        # clipboard and a folder
```

When you copy an image, xpress saves it to `~/Pictures/xpress`, optimises it, and
(on macOS) **puts the optimised image back on the clipboard** — copy large,
paste small. It doesn't re-process the image it just put back.

To run a pipeline on clipboard images, use `--pipeline`, or attach one so plain
`xpress watch` includes the clipboard:

```sh
xpress pipeline attach clipboard 'optimise'
```

The [desktop app](gui.md) offers the same on demand with **⌘⇧O**.

## Running in the background

### macOS (LaunchAgent)

Save as `~/Library/LaunchAgents/com.kwhorne.xpress.watch.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.kwhorne.xpress.watch</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/xpress</string>
    <string>watch</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <!-- lets video/audio use the app's ffmpeg -->
    <key>XPRESS_BIN_DIR</key><string>/Applications/xpress.app/Contents/Resources/bin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/xpress-watch.log</string>
  <key>StandardErrorPath</key><string>/tmp/xpress-watch.log</string>
</dict>
</plist>
```

```sh
launchctl load ~/Library/LaunchAgents/com.kwhorne.xpress.watch.plist     # start (and at login)
launchctl unload ~/Library/LaunchAgents/com.kwhorne.xpress.watch.plist   # stop
```

With no folders in `ProgramArguments`, the watcher uses your attached
automations — change them with `xpress pipeline attach/detach` and restart.

### Linux (systemd user service)

Save as `~/.config/systemd/user/xpress-watch.service`:

```ini
[Unit]
Description=xpress watch

[Service]
ExecStart=/usr/local/bin/xpress watch
Restart=on-failure

[Install]
WantedBy=default.target
```

```sh
systemctl --user enable --now xpress-watch
journalctl --user -u xpress-watch -f      # follow its output
```
