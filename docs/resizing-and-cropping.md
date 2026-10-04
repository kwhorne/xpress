# Resizing and cropping

Smaller dimensions are often the biggest saving of all: a 4032×3024 phone photo
shown at 1600 pixels wide wastes most of its bytes. Both commands below work on
**images and videos**, optimise the result, and replace the original (with a
backup) unless you give `-o`.

## Downscale by a factor

```sh
xpress downscale photo.jpg                # half the width and height (default 0.5)
xpress downscale -f 0.75 -r ~/Screenshots # 75%
xpress downscale -f 0.5 recording.mov     # videos too
```

`-f, --factor` is between `0.05` and `1.0`. The aspect ratio is kept.

## Crop or resize to a size

`xpress crop --size <SIZE>` understands several forms:

| `--size` | Result |
|----------|--------|
| `1200x630` | Exactly 1200×630: scaled to cover the area, then the overflow cropped off (good for social cards) |
| `1600x0` | 1600 wide, height follows the aspect ratio (no crop) |
| `0x720` | 720 high, width follows the aspect ratio (no crop) |
| `16:9` | The largest 16:9 area of the image, without scaling |
| `2000` with `-l, --long-edge` | The **longer** side becomes 2000; aspect kept (portrait and landscape alike) |
| `2000` | 2000×2000, cropped to a square |

```sh
xpress crop --size 1200x630 og-image.png       # social preview card
xpress crop --size 16:9 -r ~/Slides            # 16:9 crops
xpress crop --size 2000 --long-edge -r ~/Export # fit within 2000 px
```

Sizes are in **display** orientation: a portrait iPhone photo is treated as
portrait even though it's stored sideways.

### Smart crop

When a crop has to cut something off, it normally keeps the **centre**.
`--smart-crop` keeps the most **interesting** part instead — xpress scores the
image for detail, colourful subjects and skin tones and places the crop over
the best area:

```sh
xpress crop --size 1:1 --smart-crop portrait.jpg
```

Smart crop applies to images; videos are always cropped around the centre.

## Interactive crop (app)

In the app, choose **Crop image…** in the sidebar (or right-click a result →
*Crop…*), drag a rectangle over the part to keep and press **Apply crop**. See
[The desktop app](gui.md#crop-image).

## In pipelines

```text
crop(width: 1600)                  # or height:, longEdge:, ratio: 16:9, smart: true
downscale(factor: 50%)
```

See [Pipelines](pipelines.md).

## Notes

- Animated GIFs can't be resized or cropped yet; xpress stops rather than
  keeping only the first frame.
- Video dimensions are always rounded to even numbers (required by the codec).
- The result is optimised in its format afterwards (videos become MP4).
