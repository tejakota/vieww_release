# viewwsite

viewwstudio's product page, as a vieww widget tree on a `<canvas>`.

```sh
# the page, rasterised on the host — no browser, no wasm toolchain
cargo run -p viewwsite --example render          # -> site-shots/{desktop,tablet,phone}.png

# does a wheel notch move it?
cargo run -p viewwsite --example scrolltest

# the real thing
VIEWW_REPO=owner/name apps/viewwsite/build-site.sh   # -> apps/viewwsite/dist/
```

## What is in it

| file | what it is |
|---|---|
| `src/lib.rs` | the page. `root()` builds the tree; `start()` is what wasm-bindgen calls |
| `index.html` | one `<canvas>`, one `<script type="module">`, and the page's copy as the canvas's fallback content |
| `build-site.sh` | cargo → wasm-bindgen → a directory a static host can serve |
| `examples/render.rs` | the same tree to PNG on the host, at three widths |
| `examples/scrolltest.rs` | one wheel notch through `FrameDriver::handle_scroll` |

Everything on screen is drawn by vieww. The only HTML is the canvas, and the
only CSS sizes it. That is the argument the page makes, so the page had better
be made that way.

## The one line to change

`REPO` in `src/lib.rs` and `__REPO__` in `index.html` — `build-site.sh` fills
both from `VIEWW_REPO`. Every download URL is
`releases/latest/download/<name>`, against the names `packaging/package.sh`
publishes, so a release with those assets makes every link work at once. Until
then the links 404 on a placeholder repository, which is the intended failure:
a wrong-but-plausible repository is worse.

The asset table's sizes are the ones `packaging/package.sh` produces, marked
approximate, and every row is a `Pending` chip. Delete `size` and the chip once
a release publishes real numbers.

## Serving it

Any static host. Two headers matter:

- `.wasm` → `Content-Type: application/wasm`, or the browser will not
  stream-compile it;
- `.wasm` → `Content-Encoding: br` or `gzip` — 2.7 MB becomes about 900 KB.

On Cloudflare Workers, `[site] bucket = "apps/viewwsite/dist"` (or a Pages
project pointed at the same directory) does both by default.

## What a canvas costs, and what is done about it

Text drawn here is not selectable, the browser's find bar cannot see it, and a
crawler reads nothing. So `index.html` carries the page's real copy inside the
`<canvas>` element, where it is the fallback content: a screen reader reads it,
a crawler indexes it, and a browser with WebAssembly turned off shows it. The
`<title>`, the description and the Open Graph tags are on the host page for the
same reason.

That is the honest arrangement rather than a workaround. If the page's copy
changes, change it in both places — they are two renderings of one text, and
the fallback silently going stale is the failure mode to watch for.

## Status

Built and run in Chromium on 2026-09-06: it mounts, lays out, scrolls under a
wheel and a drag, re-lays out on a window resize, its buttons and table rows
navigate, its rows light under the pointer, the cursor becomes a hand over
anything tappable, and the counter in the hero counts. It is the application
that took `vieww-platform-web` from never-compiled to running — see that
crate's status section for what the first compile found and what only running
it could have.

## The motion, and what each piece is for

- **One entrance**, a second long, `EASE_OUT`. Every element that fades and
  lifts is a slice of that single `Animation<f32>` (see `stage` and `enter`),
  not an animation of its own: one ticker to poll instead of eight. The lift is
  a transform, so none of those sixty frames relayouts the page.
- **The counter in the hero** is the demo. It is the screen the code sample
  further down builds, running: a `Signal<i32>`, a `Button`, and a `Device`
  widget that reads the signal — so a tap rebuilds a phone-sized subtree and
  leaves the tables, the code blocks and the eight cards alone.
- **Rows light under the pointer** through `Pressable::sensed`, which reports
  hover as well as press, into an `AnimatedContainer` that crossfades over
  120 ms — a fill that snaps looks like a flicker when the pointer crosses a
  row on its way somewhere else.
- **Nothing else moves.** The page presents a frame only when the scene was
  rebuilt, so it is idle at zero frames a second between interactions.

Not yet done: `prefers-color-scheme` (the page is dark-only), a keyboard focus
order through the tappable rows, and the device-pixel scale, which needs a root
transform on `NativeRenderer::render_to_pixels` — see `WebSurface::resize`.
