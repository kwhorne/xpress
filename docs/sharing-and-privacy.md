# Sharing and privacy

## Fit a file where it's going: `--for`

Chat apps, code hosts and email have upload limits and don't show every format.
`--for` makes a file fit a destination in one step:

```sh
xpress optimise --for discord clip.mov
xpress optimise --for github screenshot.webp demo.mov
xpress optimise --for email IMG_0042.HEIC
```

It first **converts** formats the destination can't show (into a new file next
to the original), then **compresses** to the destination's size limit — video
and audio by computing the bitrate, images and PDFs by raising the compression.

| `--for` | Size xpress aims for | The service's limit | Format changes |
|---------|---------------------|---------------------|----------------|
| `discord` | 19 MB per file | 20 MB per file (free accounts) | — |
| `github` | 9.5 MB images & video, 24 MB other files | 10 MB images, GIFs and videos (free plans); 25 MB other files | WebP, AVIF, HEIC, JPEG XL, BMP, TIFF → JPEG (PNG if transparent): GitHub shows only PNG, GIF, JPEG and SVG |
| `email` | 14 MB per file | Gmail 25 MB, Outlook 20 MB per message — attachments grow by about a third when sent | HEIC, AVIF, JPEG XL → JPEG (PNG if transparent) |

xpress aims a little under each limit to be safe. The limits were checked in
September 2026; services change them, and xpress is updated when they do.
A file that can't be made small enough is kept at its smallest, with a warning.

Example — a 20-second 1080p screen recording (234 MB) came out at 18.8 MB for
Discord, 9.1 MB for GitHub and 13.5 MB for email.

## Remove where a photo was taken: `--strip-location`

Photos from phones usually record **GPS coordinates**. Before sharing, remove
just that and keep everything useful:

```sh
xpress optimise --strip-location IMG_0042.jpg
xpress optimise --for email --strip-location IMG_0042.HEIC
```

In the app: Preferences → **Remove location**.

What's removed:

- the EXIF **GPS** block — wiped from the file, not just unlinked, so the
  coordinates aren't left behind in the bytes;
- location fields in **XMP** (GPS, city, state, country, location);
- the recording location of **videos**.

What's kept: camera and lens, date and time, orientation, colour profile,
ratings, captions and other XMP.

## Remove all metadata: `--strip-metadata`

```sh
xpress optimise --strip-metadata -r ~/Export
```

Removes all **EXIF** and **XMP** from images (camera, date, location, edit
history…) and all global metadata from videos. Two things are deliberately kept
so images still look right: the **colour profile** and the correct
**orientation** (applied to the pixels). In the app: Preferences →
**Strip metadata**.

There's also `xpress strip-exif <files>`, which removes metadata *without
re-encoding* the image; it needs [`exiftool`](https://exiftool.org).

## What xpress keeps by default

Without these options, xpress keeps a photo's metadata when it optimises or
converts it — including location. Converting to AVIF can't keep the colour
profile (the pixels are converted to sRGB instead), and GIF and BMP can't store
metadata at all.

## Clipboard history

The app's [clipboard history](history.md) is **off until you turn it on**,
keeps everything on your Mac (`history/` in the config folder), never records
content that password managers mark as private, and ignores copies made in
password managers. Text recognition in images runs on the Mac too. *Paste directly* uses the
Accessibility permission only to press ⌘V in the app you return to. Apple
Intelligence runs on the Mac's own model. *Sync with iCloud* (off by default)
stores the history in your iCloud Drive — end-to-end encrypted when Advanced
Data Protection is on; see [Sync between your Macs](history.md#sync-between-your-macs). Delete single
clips with right-click → *Delete*, or everything unpinned with *Clear…* in
Preferences.

## Copy large, paste small

To share screenshots quickly: copy an image, press **⌘⇧O** (app running), then
paste — you paste the optimised version. The command-line equivalent is
`xpress watch --clipboard` ([Watching](daemon.md)).
