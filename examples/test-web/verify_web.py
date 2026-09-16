#!/usr/bin/env python3
"""Verify the web backend byte for byte against the native rasteriser.

    python3 examples/test-web/verify_web.py <dist-dir> <baseline-dir> [out-dir]

`<dist-dir>` is what `examples/test-web/build-web.sh` produces (index.html,
test_web.js, test_web_bg.wasm). `<baseline-dir>` is what
`cargo run --release -p test-web --example baseline -- <dir>` produces
(native-baseline.rgba, native-tapped.rgba).

What it does, in order:

1. serves `<dist-dir>` over HTTP (the .wasm with `application/wasm`),
2. opens it in headless Chromium at devicePixelRatio 1,
3. waits for `#boot` to read `mounted` and for the canvas to stop changing,
4. reads the canvas back with `getImageData` and compares it with
   `native-baseline.rgba` — **byte for byte**,
5. clicks the canvas, waits for it to settle again, and compares the result
   with `native-tapped.rgba`.

Prints `web.baseline=EQUAL|DIFFERENT(n bytes)`, `web.tapped=...`, and writes
`web-<state>.rgba` into `[out-dir]` for inspection. Exit code 0 only when both
are equal. Exit code 3 when Playwright/Chromium is unavailable — a machine
skip, which the certification script records as a skip rather than a pass.

This file was referenced by `examples/test-web/build-web.sh` as living "in the
project that certified this", i.e. outside the repository — which made the web
certification unrepeatable from a checkout. It lives here now.
"""

from __future__ import annotations

import functools
import http.server
import os
import socketserver
import sys
import threading
import time
from pathlib import Path

WIDTH, HEIGHT = 960, 600


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    dist = Path(sys.argv[1]).resolve()
    baseline = Path(sys.argv[2]).resolve()
    out = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else baseline
    out.mkdir(parents=True, exist_ok=True)

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("web.verify=MACHINE-SKIPPED(python playwright not installed)")
        return 3

    for name in ("index.html", "test_web.js", "test_web_bg.wasm"):
        if not (dist / name).exists():
            print(f"web.verify=FAIL(missing {dist / name}; run build-web.sh)")
            return 1
    expected = {}
    for state in ("baseline", "tapped"):
        path = baseline / f"native-{state}.rgba"
        if not path.exists():
            print(f"web.verify=FAIL(missing {path}; run the baseline example)")
            return 1
        expected[state] = path.read_bytes()

    handler = functools.partial(QuietHandler, directory=str(dist))
    with socketserver.TCPServer(("127.0.0.1", 0), handler) as server:
        port = server.server_address[1]
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with sync_playwright() as p:
                try:
                    browser = p.chromium.launch(headless=True)
                except Exception as error:  # noqa: BLE001 - report any launch failure
                    print(f"web.verify=MACHINE-SKIPPED(chromium failed to launch: {error})")
                    return 3
                page = browser.new_page(
                    viewport={"width": WIDTH + 80, "height": HEIGHT + 80},
                    device_scale_factor=1,
                )
                errors: list[str] = []
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.goto(f"http://127.0.0.1:{port}/index.html")
                page.wait_for_function(
                    "document.getElementById('boot').textContent === 'mounted'",
                    timeout=60_000,
                )
                ok = True
                first = settle(page)
                ok &= compare("baseline", first, expected["baseline"], out)
                box = page.locator("#vieww-canvas").bounding_box()
                page.mouse.click(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
                second = settle(page, previous=first)
                ok &= compare("tapped", second, expected["tapped"], out)
                if errors:
                    print("web.page_errors=" + " | ".join(errors))
                    ok = False
                browser.close()
        finally:
            server.shutdown()
    print(f"web.verify={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def settle(page, previous: bytes | None = None, timeout: float = 30.0) -> bytes:
    """Read the canvas until two reads 250 ms apart agree (and, if given,
    differ from `previous`)."""
    deadline = time.monotonic() + timeout
    last = read_canvas(page)
    while time.monotonic() < deadline:
        time.sleep(0.25)
        now = read_canvas(page)
        if now == last and (previous is None or now != previous):
            return now
        last = now
    return last


def read_canvas(page) -> bytes:
    data = page.evaluate(
        """() => {
            const c = document.getElementById('vieww-canvas');
            const ctx = c.getContext('2d');
            return Array.from(ctx.getImageData(0, 0, c.width, c.height).data);
        }"""
    )
    return bytes(data)


def compare(state: str, actual: bytes, wanted: bytes, out: Path) -> bool:
    (out / f"web-{state}.rgba").write_bytes(actual)
    if len(actual) != len(wanted):
        print(f"web.{state}=DIFFERENT(size {len(actual)} != {len(wanted)})")
        return False
    differing = sum(1 for a, b in zip(actual, wanted) if a != b)
    print(f"web.{state}=" + ("EQUAL" if differing == 0 else f"DIFFERENT({differing} bytes)"))
    return differing == 0


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def log_message(self, format, *args):  # noqa: A002 - signature is inherited
        pass


if __name__ == "__main__":
    os.environ.setdefault("PLAYWRIGHT_BROWSERS_PATH", "/opt/pw-browsers")
    sys.exit(main())
