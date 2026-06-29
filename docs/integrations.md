# Integrations

xpress is a CLI + engine, so it composes with the OS rather than embedding
platform frameworks. Here is how to wire it into common workflows.

## macOS Shortcuts

xpress integrates with the **Shortcuts** app via the **“Run Shell Script”**
action — no extra build or signing needed.

1. In Shortcuts, add **Run Shell Script** (input: *Shortcut Input as arguments*,
   or pass file paths).
2. Call the CLI, e.g.:

   ```sh
   /usr/local/bin/xpress optimise --aggressive "$@"
   # or a saved pipeline:
   /usr/local/bin/xpress pipeline run web "$@"
   ```

3. Use it from the Share sheet, Quick Actions (Finder right-click), or Automator
   “Folder Actions”. For Finder Quick Actions, wrap the same command in an
   Automator **Quick Action** that receives image/video files.

Tip: `xpress … --json` makes the output easy to parse in later Shortcuts steps.

## Folder Actions (auto-optimise a folder)

Either use the built-in watcher:

```sh
xpress pipeline attach ~/Screenshots 'crop(longEdge: 2000) -> convert(to: webp)' --type image
xpress watch
```

…or a macOS **Folder Action** / `launchd` agent that runs `xpress optimise` on
new files, if you prefer the OS to own the lifecycle.

## Photos

Direct Photos library access needs a signed app with PhotoKit entitlements,
which is outside a CLI's scope. The practical flow:

1. Drag/export the photos out of Photos (or use a Shortcut’s “Get Latest Photos”).
2. Run `xpress optimise` / a pipeline on the exported files.

A Shortcut can chain *Get Photos → Save to folder → Run Shell Script (xpress)*.

## Uploads / “copy link for sending”

Uploading is provider-specific, so xpress exposes it through the generic
`runScript` pipeline step (the current file is in `$FILE`):

```sh
# Optimise then upload via your own tool, and copy the URL to the clipboard.
xpress pipeline run \
  'convert(to: webp) -> runScript(code: "url=$(myuploader \"$FILE\"); printf %s \"$url\" | pbcopy")' \
  screenshot.png
```

Swap `myuploader` for `scp`, `rclone`, `aws s3 cp`, a curl call, etc. Because
`runScript` runs after the optimisation steps, the uploaded file is the
optimised one.

## Drag-and-drop out of the GUI

The desktop app shows **Reveal** and **Copy** on each result card. Native
drag-*out* into other apps is not currently supported by the egui shell; use
**Copy** (puts the image on the clipboard) or **Reveal** (opens it in the file
manager) and drag from there. A future native shell could add true drag-out.
