#!/usr/bin/env python3
"""Every failed row of one or more gate runs, with the end of its log, as Markdown.

    ci/vieww failures RUN_DIR [RUN_DIR...] [-o FAILURES.md]

Written for places where the run folder itself is out of reach — a GitHub job
summary, or a checklist artifact that is easier to open than the run folders.
For each FAIL row it prints the last lines of that row's log; for a failed
certification (G1.3) it also names every failed stage inside it with the end of
that stage's own log, and for a failed `checks` (G1.1) the failing stage names.

Standard library only, so it runs on every OS a gate runs on.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

TAIL = 60          # lines of log per failure
WIDTH = 400        # characters per line, so one minified blob cannot flood a summary


# Cargo colours its output on CI (`CARGO_TERM_COLOR=always`), which puts an
# escape code between the start of the line and `error`, so none of the
# `^error` patterns below matched and the clippy diagnostics never reached the
# summary — the first macOS and Windows reports said "FAILED: clippy" and
# nothing about why. Strip colour before matching anything.
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\([A-Z]")


def read_lines(path: pathlib.Path) -> list[str]:
    return ANSI.sub("", path.read_text(encoding="utf-8", errors="replace")).splitlines()


def tail(path: pathlib.Path, n: int = TAIL) -> str:
    if not path.is_file():
        return f"(no log at {path.name})"
    lines = read_lines(path)
    # Cargo's progress lines say nothing about why something failed.
    lines = [l for l in lines if not re.match(r"^\s+(Compiling|Checking|Documenting|Downloaded|Downloading|Fresh|Blocking) ", l)]
    cut = lines[-n:]
    body = "\n".join(l[:WIDTH] for l in cut)
    more = f"… {len(lines) - n} earlier lines omitted\n" if len(lines) > n else ""
    return more + body


ERROR = re.compile(
    r"panicked at|^---- .* stdout ----|^error(\[[^\]]+\])?:|^warning\[|test result: FAILED|"
    r"^failures:|FAILED|No such file|not found|denied|refused|Segmentation|signal \d+|"
    r"(advisories|bans|licenses|sources) (ok|FAILED)"
)


# A passing test whose *name* happens to contain "refused" or "not_found" is
# not an error. Without this, a workspace full of `..._is_refused ... ok` tests
# used the whole excerpt budget and pushed the real clippy and build errors out
# of the summary — the macOS and Windows G1.1 reports named "FAILED: clippy"
# and then listed thirty passing tests instead of the lint.
PASSED = re.compile(r"^test .* \.\.\. (ok|ignored)\b")


def excerpt(path: pathlib.Path, limit: int = 90) -> str:
    """The error-shaped lines of a log, each with a little context after it.

    A tail shows how a log ended, which for `cargo test` is a summary and for
    `cargo deny` is nine thousand lines of dependency tree. The cause is earlier:
    a panic message, an `error[...]` header, a failing test's name.
    """
    if not path.is_file():
        return ""
    lines = read_lines(path)
    keep: list[str] = []
    skip_until = -1
    for i, line in enumerate(lines):
        if i <= skip_until or not ERROR.search(line) or PASSED.match(line):
            continue
        # An error header's explanation follows it; an inclusion graph does not help.
        # Plain `error:` too: clippy's lints print as `error: <message>` with
        # the `--> file:line` on the next line, and without that line the
        # summary says a lint failed but not where.
        after = 6 if re.match(r"^(error|warning)(\[|:)|.*panicked at|^---- ", line) else 0
        chunk = [l for l in lines[i : i + 1 + after] if not re.match(r"^\s*[│├└]", l)]
        keep.extend(l[:WIDTH] for l in chunk)
        keep.append("")
        skip_until = i + after
        if len(keep) >= limit:
            keep.append("… more errors in the full log")
            break
    return "\n".join(keep).strip()


def block(title: str, text: str) -> str:
    return f"<details><summary>{title}</summary>\n\n```text\n{text.replace('```', '` ` `')}\n```\n\n</details>\n"


def report(run: pathlib.Path) -> tuple[str, int]:
    rows_path = run / "rows.tsv"
    if not rows_path.is_file():
        return f"### `{run.name}`\n\nNot a gate run (no rows.tsv).\n", 0
    out = [f"### `{run.name}`\n"]
    failures = 0
    for line in rows_path.read_text(encoding="utf-8").splitlines():
        parts = line.split("\t") + ["", "", "", ""]
        row_id, what, verdict, evidence = parts[:4]
        if not verdict.upper().startswith("FAIL"):
            continue
        failures += 1
        out.append(f"#### ❌ {row_id} — {what}\n\n`{verdict}`\n")
        if evidence.startswith("logs/"):
            ex = excerpt(run / evidence)
            if ex:
                out.append(block(f"errors in {evidence}", ex))
            out.append(block(f"end of {evidence}", tail(run / evidence, 25)))
        if row_id == "G1.1":
            log = run / "logs" / "G1.1.txt"
            if log.is_file():
                text = read_lines(log)
                failed = [l.strip()[len("FAILED:"):].strip() for l in text if l.strip().startswith("FAILED:")]
                if failed:
                    out.append("Failed stages in `checks`:\n\n" + "\n".join(f"- {l}" for l in failed[:20]) + "\n")
                # A stage's error is in the middle of a long log, not at its end:
                # pull the error-shaped lines out of each failed stage's section.
                sections: dict[str, list[str]] = {}
                current = ""
                for l in text:
                    if l.startswith("==> "):
                        current = l[4:].strip()
                        sections[current] = []
                    elif current:
                        sections[current].append(l)
                pattern = re.compile(r"error(\[|:)|panicked|FAILED|failures:|test result: FAILED|^---- |fatal|denied|rejected|no such|not found", re.I)
                for stage, body in sections.items():
                    if not any(stage.startswith(f.split(" (exit")[0]) for f in failed):
                        continue
                    hits = []
                    for n, l in enumerate(body):
                        if not pattern.search(l) or PASSED.match(l):
                            continue
                        hits.append(l[:WIDTH])
                        # Keep a diagnostic's location and the source line it points at.
                        if re.match(r"^(error|warning)(\[|:)", l):
                            hits.extend(x[:WIDTH] for x in body[n + 1 : n + 4] if x.strip())
                    hits = hits[:120]
                    if hits:
                        out.append(block(f"errors in stage: {stage}", "\n".join(hits)))
        if row_id == "G1.3":
            stages = run / "cert" / "quality" / "stages.txt"
            if stages.is_file():
                for s in stages.read_text(encoding="utf-8").splitlines():
                    m = re.match(r"^(\S+)\s+(FAIL.*)$", s)
                    if not m:
                        continue
                    name, v = m.groups()
                    rel = name.replace("\\", "/")
                    out.append(f"- certification stage **{rel}**: `{v}`\n")
                    log = run / "cert" / f"{rel}.txt"
                    if log.is_file():
                        ex = excerpt(log)
                        if ex:
                            out.append(block(f"errors in cert/{rel}.txt", ex))
                        out.append(block(f"end of cert/{rel}.txt", tail(log, 25)))
    if failures == 0:
        out.append("No failed rows.\n")
    return "\n".join(out), failures


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="+", type=pathlib.Path)
    ap.add_argument("-o", "--output", type=pathlib.Path)
    args = ap.parse_args()
    parts = ["## Failures, with logs\n"]
    total = 0
    for run in args.runs:
        text, n = report(run)
        parts.append(text)
        total += n
    md = "\n".join(parts)
    if args.output:
        args.output.write_text(md, encoding="utf-8")
    else:
        sys.stdout.write(md)
    print(f"{total} failed row(s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
