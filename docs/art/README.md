# README and social artwork

The banner, the close-ups, the architecture panel and the social preview at the top of the
[README](../../README.md) are HTML pages rendered to PNG. Edit a page here, then render:

```sh
npm install --no-save playwright && npx playwright install chromium   # once
node tools/art/render.mjs              # every page, into docs/assets/art/
node tools/art/render.mjs hero         # one page
```

| Page | Output | Used in |
|---|---|---|
| `hero.html` | `hero.png` | top of the README |
| `feature-*.html` | `feature-*.png` | README › A closer look |
| `architecture.html` | `architecture.png` | README › Under the hood |
| `social-preview.html` | `social-preview.png` (1280×640) | Settings › General › Social preview, uploaded by hand |

`art.css` holds the shared look: colours from the app's `crates/dstudio-app/ui/tokens.slint` and the app's
bundled Inter font. `mark.svg` is the mark: one cycle of a sine wave on the app's blue.

## The app views

`shots/` holds the app views the pages frame. They are rendered by the app itself at 2x, with the Linux
menu bar (the top 88 pixels) cropped off:

```sh
SLINT_SCALE_FACTOR=2 SLINT_WGPU_CPU=1 WGPU_BACKEND=vulkan xvfb-run -a -s "-screen 0 3200x2000x24" \
  target/release/device-studio --no-audio --zoom 85 --state review-automation \
  --screenshot automation.png --exit-after-frames 5
```

| Shot | `--state` |
|---|---|
| `automation.png` | `review-automation` |
| `analyzers.png` | `review-analyzers` |
| `lfo.png` | `reference-lfo` |

`--zoom 85` makes the interface scale exactly 2, so the layout matches a 1440×820 window. When a view changes
visibly, render its shot again, crop it the same way and re-run `render.mjs`.
