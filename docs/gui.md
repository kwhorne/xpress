# Desktop GUI

`xpress-gui` is a small native app (egui/eframe) for interactive optimisation.

```sh
cargo run -p xpress-gui --release
```

## Features

- **Drag and drop** images, videos, PDFs or audio onto the window to optimise them.
- **Result cards** show before/after size, the saving (`-NN%`), and a thumbnail
  for images.
- **Global hotkey** `⌘⇧O` (macOS) / `Ctrl⇧O` optimises the image currently on the
  clipboard, from anywhere.
- **Controls**: a compression slider (5–100), `aggressive`, `backup`,
  `strip metadata`, an inline **pipeline** field, and a **float on top** toggle.
- **Open files…** picker, and **Optimise clipboard** / **Clear** buttons.
- **Crop image…** opens an interactive crop tool: drag a region and **Apply crop**.
- Each result card has **Reveal** (show in the file manager) and **Copy** (put the
  image on the clipboard). Native drag-*out* isn't supported by the egui shell
  yet — see [integrations](integrations.md).
- Work runs off the UI thread, so the window stays responsive while encoding.

Clipboard images are saved to `~/Pictures/xpress`.

## Building a macOS `.app`

```sh
cargo build --release -p xpress-gui -p xpress-cli
scripts/make-app.sh                 # -> dist/xpress.app  (ad-hoc signed)
scripts/make-app.sh --tools         # also bundle ffmpeg/pngquant/... inside
scripts/make-dmg.sh                 # -> dist/xpress.dmg  (drag-to-Applications)
```

The bundle is ad-hoc signed so it runs locally and includes the app icon from
`assets/AppIcon.icns`. Tagged releases publish both `xpress-*-app.zip` and a
`xpress-*.dmg` disk image (with an Applications shortcut for drag-installing).

### Distribution (Developer ID + notarisation)

For sharing outside your own machine, sign and notarise (commands are at the
bottom of `scripts/make-app.sh`):

```sh
codesign --force --options runtime --timestamp \
  --sign "Developer ID Application: Your Name (TEAMID)" dist/xpress.app
ditto -c -k --keepParent dist/xpress.app xpress.zip
xcrun notarytool submit xpress.zip --apple-id you@example.com \
  --team-id TEAMID --password APP_SPECIFIC_PWD --wait
xcrun stapler staple dist/xpress.app
```

## App icon

The icon is defined as vector art in `assets/icon.svg` and rendered to
`assets/AppIcon.icns`. To regenerate after editing the SVG:

```sh
cargo run --manifest-path tools/icon-gen/Cargo.toml --release -- \
  assets/icon.svg assets/xpress.iconset
iconutil -c icns assets/xpress.iconset -o assets/AppIcon.icns
```
