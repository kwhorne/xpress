# Clipboard and screenshot history

![The History view](history.png)

The desktop app can keep **everything you copy and every screenshot you take**,
so you can find it again later and copy it back — text, links, code, colours,
images and files. Words *inside* images are searchable too: xpress reads the
text in screenshots and copied images.

History is **off until you turn it on**, stays on your Mac, and never records
passwords.

## Turning it on

Open **History** in the sidebar and click **Turn on clipboard history** — or
switch on *Clipboard history* in **Preferences**. From then on, xpress records:

- **What you copy** in any app: text, links, code, colours, images, and files
  copied in Finder.
- **New screenshots** (⌘⇧3, ⌘⇧4, ⌘⇧5), from the folder macOS saves them to.
  Screenshots copied straight to the clipboard (⌃⌘⇧4) are recorded as images.

Only what you copy *after* turning it on is recorded; your existing files and
screenshots aren't imported.

## Finding and pasting

Press **⌃⌘V** in any app (or choose *Clipboard history* from the ✕ menu). The
history opens with the cursor in the search field:

1. **Type** a few letters — words match by prefix and in any order, so
   `inv 47` finds “Invoice 4711”. Accents don't matter (`cafe` finds “café”).
2. **↑ / ↓** to choose, **⏎** to copy. xpress hides itself and puts you back in
   the app you came from — press **⌘V** to paste.
3. **Esc** clears the search, or closes the window when it's empty.

Double-clicking a clip does the same as ⏎; **Copy** on a row copies it and
leaves the window open.

The search covers the text of each clip, the **text recognised in images**,
file names and paths, and the name of the app it came from.

### Filters

Under the search field:

| Filter | Shows |
|--------|-------|
| **All** | Everything, most recently used first |
| **Pinned** | Clips you've pinned |
| **Text**, **Links**, **Code**, **Colours** | Copied text, sorted by what it looks like |
| **Images**, **Screenshots** | Copied images; screenshots from the screenshot folder |
| **Files** | Files and folders copied in Finder |
| **All apps ▾** | Only clips copied from one app |

Each row shows a preview (a thumbnail, a colour swatch, or an icon), the first
line, and *kind · app · when · size*. Copying a clip again moves it to the top.

### Right-click a clip

| Item | Does |
|------|------|
| Copy | Put it on the clipboard |
| Copy text in image | Copy the words recognised in a screenshot or image |
| Open link | Open a link clip in your browser |
| Show in Finder | Reveal a copied file, or the stored image |
| Pin / Unpin | Pinned clips are never removed automatically (also ☆ on the row) |
| Delete | Remove it from the history |

## What's recorded, and what isn't

xpress decides what a copy is, in this order:

- **Files** copied in Finder are kept as a list of paths (with a preview for a
  single image). The files themselves aren't copied into the history.
- **Text** wins over a picture of the same content (Word, Excel and Keynote put
  both on the clipboard).
- **Images** — unless the only text alongside is the image's address, which is
  what browsers add when you *Copy Image*.
- Text is sorted into **links** (`https://…`, `www.…`, `mailto:`), **colours**
  (`#7aa2f7`, `rgb(…)`, `hsl(…)`), **code** (by its shape: braces, semicolons,
  indentation, keywords) or plain **text**.

The same content copied twice is one clip — it moves back to the top. Images
count as the same when their pixels are.

**Never recorded:**

- Anything an app marks as **private or temporary** on the clipboard — password
  managers do this for passwords and one-time codes (the
  [nspasteboard.org](http://nspasteboard.org) markers, and 1Password's own).
- Anything copied while **Passwords, Keychain Access, 1Password, Bitwarden,
  KeePassXC or Dashlane** is in front.
- Text over 5 MB.

## Text in images (OCR)

With *Find text in images* on (the default), xpress reads the words in every
new screenshot and copied image with macOS's built-in text recognition. It runs
**on your Mac** — nothing is uploaded — and recognises many languages. The
words appear as the clip's title (“Invoice 4711 …”), are searchable, and can be
copied with *Copy text in image*.

## How long it's kept

*Keep history* in Preferences: 1 day, 1 week, **1 month** (default), 3 months,
1 year or forever. Older clips are removed about once an hour, oldest first;
**pinned clips are always kept**. Whatever you choose, the history never grows
past **2 GB** — the oldest unpinned clips go first.

**Clear…** in Preferences deletes every clip except pinned ones (click twice to
confirm). To delete a single clip, right-click it → *Delete*.

## Where it's stored

In the `history` folder next to the config files —
`~/Library/Application Support/xpress/history` on macOS:

| | |
|---|---|
| `history.db` | The clips and the search index (SQLite) |
| `images/` | Copied images and screenshots; PNGs are recompressed losslessly, so they take less room than the originals |
| `thumbs/` | Small previews |

Screenshots are **copied** into the history, so deleting the original from your
Desktop doesn't remove it from the history (and vice versa). Turning the history
off stops recording; the clips stay until you clear them.

## Settings

All in **Preferences → Clipboard history**:

| Setting | Default | |
|---------|---------|---|
| Clipboard history | off | Record what you copy |
| Include screenshots | on | Also record new screenshots |
| Find text in images | on | Recognise words in images (macOS) |
| Keep history | 1 month | How long unpinned clips are kept |
| Clear… | | Delete all unpinned clips |

## Platforms

Everything above is for **macOS**. On Linux the app records copied **text**
only, without the source app, and there's no text recognition.
