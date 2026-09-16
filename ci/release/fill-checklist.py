#!/usr/bin/env python3
"""Fill docs/release/BETA-RELEASE-CHECKLIST.md from one or more gate runs.

    ci/vieww checklist RUN_DIR [RUN_DIR...] [-o CHECKLIST.md]

Each RUN_DIR is a `ci/vieww gate` output folder (it holds rows.tsv and
host.txt). Pass the runs from every machine, for example linux-gpu,
linux-cpu, windows-gpu and macos-gpu, to get one checked checklist.

Rules:

* Only cells that are still `☐` are filled. N/A cells and the free-text
  columns are left alone.
* Matrix columns are matched to runs by OS, and by mode where the column
  names one: L-CPU, W-GPU and so on. A column naming only the OS (Linux, L)
  takes every run for that OS. A `Result` column takes every run.
* When several runs cover one cell, FAIL beats PASS, PASS beats SKIPPED.
* Rows no run measured stay `☐`, so manual rows are still visibly open.
* Nothing here approves a release. The final line can at most say
  "READY FOR SIGN-OFF".

Uses only the Python standard library, so it runs on Linux, macOS and
Windows (Git Bash `python`).
"""

from __future__ import annotations

import argparse
import datetime as _dt
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TEMPLATE = ROOT / "docs" / "release" / "BETA-RELEASE-CHECKLIST.md"

OS_NAMES = {"linux": "Linux", "windows": "Windows", "macos": "macOS"}
OS_BY_HEADER = {
    "linux": "linux", "l": "linux",
    "windows": "windows", "w": "windows",
    "macos": "macos", "m": "macos",
}
# Aggregation priority: the first class present wins.
PRIORITY = ["fail", "pass", "warn", "skip", "info", "na"]
SYMBOL = {
    "fail": "❌ FAIL",
    "pass": "✅ PASS",
    "warn": "⚠️ SOFTWARE GPU",
    "skip": "⏭️ SKIPPED",
    "info": "📝 RECORDED",
    "na": "N/A",
}


def classify(verdict: str) -> str:
    v = verdict.strip().upper()
    if v.startswith("PASS"):
        return "pass"
    if v.startswith("FAIL"):
        return "fail"
    if v.startswith("SOFTWARE"):
        return "warn"
    if v.startswith(("SKIPPED", "NOT RUN")):
        return "skip"
    if v.startswith("RECORDED"):
        return "info"
    return "na"  # N/A, BUILD-ONLY


class Run:
    def __init__(self, path: pathlib.Path):
        self.path = path
        self.host = parse_kv(path / "host.txt")
        self.os = self.host.get("os", "")
        self.rows: dict[str, tuple[str, str, str]] = {}
        for line in (path / "rows.tsv").read_text(encoding="utf-8").splitlines():
            parts = line.split("\t")
            if len(parts) >= 3:
                parts += [""] * (4 - len(parts))
                self.rows[parts[0]] = (parts[1], parts[2], parts[3])
        mode = self.host.get("mode", "auto")
        if mode == "auto":
            # An auto run is a GPU run exactly when it found real hardware.
            g14 = self.rows.get("G1.4", ("", "", ""))[1]
            mode = "gpu" if g14.upper().startswith("PASS") else "cpu"
        self.mode = mode
        self.source = parse_kv(path / "cert" / "meta" / "source.txt")
        self.manifest = parse_kv(path / "package" / "MANIFEST.txt")

    @property
    def label(self) -> str:
        return f"{self.os}-{self.mode}"


def parse_kv(path: pathlib.Path) -> dict[str, str]:
    out: dict[str, str] = {}
    if path.is_file():
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if "=" in line and not line.startswith("#"):
                key, value = line.split("=", 1)
                out.setdefault(key.strip(), value.strip())
    return out


def cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def join(parts: list[str]) -> str:
    return "| " + " | ".join(parts) + " |"


def column_target(header: str):
    """(os or None, mode or None) for a matrix header, or None if not a result column."""
    h = header.strip().lower()
    if h == "result":
        return (None, None)
    m = re.fullmatch(r"([lwm])-(cpu|gpu)", h)
    if m:
        return (OS_BY_HEADER[m.group(1)], m.group(2))
    if h in OS_BY_HEADER:
        return (OS_BY_HEADER[h], None)
    return None


def aggregate(runs: list[Run], row_id: str, os_name, mode):
    found = []
    evidence = []
    for run in runs:
        if os_name and run.os != os_name:
            continue
        if mode and run.mode != mode:
            continue
        if row_id in run.rows:
            _, verdict, ev = run.rows[row_id]
            found.append((classify(verdict), verdict))
            if ev:
                evidence.append(f"{run.path.name}/{ev}")
    if not found:
        return None
    for cls in PRIORITY:
        hits = [v for c, v in found if c == cls]
        if hits:
            return cls, hits[0], evidence
    return None


def cross_run_rows(runs: list[Run]) -> None:
    """Rows only a set of runs can answer: same source, same commit."""
    digests = {r.source.get("source_digest") for r in runs if r.source.get("source_digest")}
    if digests:
        verdict = "PASS" if len(digests) == 1 else f"FAIL({len(digests)} different source digests)"
        for r in runs:
            r.rows["G0.12"] = ("same source digest in every run", verdict, "cert/meta/source.txt")
    revs = {r.manifest.get("git_rev") for r in runs if r.manifest.get("git_rev")}
    if revs:
        ok = len(revs) == 1 and "none" not in revs
        verdict = "PASS" if ok else f"FAIL(artifacts from {', '.join(sorted(revs))})"
        for r in runs:
            if r.manifest:
                r.rows["G8.4"] = ("artifacts from the certified commit", verdict, "package/MANIFEST.txt")


def fill(template: str, runs: list[Run]) -> tuple[str, dict]:
    lines = template.splitlines()
    header: list[str] | None = None
    stats = {"pass": 0, "fail": 0, "warn": 0, "skip": 0, "open": 0}
    gate_state: dict[str, dict[str, int]] = {}
    out = []
    for line in lines:
        if not line.startswith("|"):
            header = None if not line.strip() else header
            out.append(line)
            continue
        parts = cells(line)
        if parts and parts[0] in ("ID", "Field", "Gate"):
            header = parts
            out.append(line)
            continue
        if set("".join(parts)) <= set("-: "):
            out.append(line)
            continue
        row_id = parts[0] if parts else ""
        if header and header[0] == "ID" and re.fullmatch(r"G\d+\.\d+", row_id):
            gate = row_id[1:].split(".")[0]
            state = gate_state.setdefault(gate, {"fail": 0, "open": 0})
            evidence_col = header.index("Evidence") if "Evidence" in header else None
            for i, name in enumerate(header):
                if i >= len(parts) or "☐" not in parts[i]:
                    continue
                target = column_target(name)
                if target is None:
                    continue
                result = aggregate(runs, row_id, *target)
                if result is None:
                    stats["open"] += 1
                    state["open"] += 1
                    continue
                cls, verdict, evidence = result
                text = SYMBOL[cls]
                if cls in ("fail", "warn", "info") and "(" in verdict:
                    detail = verdict[verdict.index("(") + 1 : verdict.rindex(")")] if ")" in verdict else ""
                    text += f" ({detail})" if detail else ""
                parts[i] = text
                stats[cls if cls in stats else "skip"] += 1
                if cls == "fail":
                    state["fail"] += 1
                if cls in ("skip",):
                    state["open"] += 1
                if evidence_col is not None and evidence and not parts[evidence_col]:
                    parts[evidence_col] = "<br>".join(f"`{e}`" for e in evidence)
            out.append(join(parts))
            continue
        out.append(line)
    return "\n".join(out) + "\n", {"stats": stats, "gates": gate_state}


def fill_header(text: str, runs: list[Run]) -> str:
    versions = sorted({r.host.get("version", "") for r in runs} - {""})
    revs = sorted({r.host.get("git_rev", "") for r in runs} - {""})
    if versions:
        text = re.sub(r"^\| \*\*Version\*\* \|.*\|$", f"| **Version** | {', '.join(versions)} |", text, count=1, flags=re.M)
    if revs:
        text = re.sub(r"^\| \*\*Certified commit / tag\*\* \|.*\|$", f"| **Certified commit / tag** | {'<br>'.join(revs)} |", text, count=1, flags=re.M)
    return text


def fill_platform_record(text: str, runs: list[Run]) -> str:
    def per_os(os_name: str, field: str) -> str:
        chosen = [r for r in runs if r.os == os_name]
        if not chosen:
            return ""
        vals = []
        for r in chosen:
            h = r.host
            if field == "Machine / OS version":
                v = f"{h.get('machine', '')} · {h.get('os_version', '')}"
            elif field == "Arch / CPU":
                v = f"{h.get('arch', '')} · {h.get('cpu', '')}"
            elif field == "GPU / driver / Vulkan device":
                v = " · ".join(x for x in (h.get("gpu", ""), h.get("vulkan_deviceName", ""), h.get("vulkan_driverInfo", "")) if x) or h.get("vulkan", "")
            elif field == "Display server":
                v = h.get("display_server", "")
            elif field == "Rust / Cargo":
                v = f"{h.get('rustc', '')} · {h.get('cargo', '')}"
            elif field == "Commit / source digest":
                v = f"{h.get('git_rev', '')} · {r.source.get('source_digest', '')[:16]}"
            elif field == "Gate folders (gpu, cpu)":
                v = f"`{r.path.name}`"
            elif field == "Artifacts + SHA256SUMS":
                sums = r.path / "package" / "SHA256SUMS"
                v = ", ".join(l.split()[-1] for l in sums.read_text().splitlines() if l.strip()) if sums.is_file() else ""
            elif field == "Signing status":
                v = r.manifest.get("signing", "")
            elif field == "**Verdict**":
                fails = sum(1 for _, verdict, _ in r.rows.values() if classify(verdict) == "fail")
                v = f"{r.mode}: auto {'❌ ' + str(fails) + ' failed' if fails else '✅'}"
            else:
                return ""
            v = v.strip(" ·")
            if v:
                vals.append(f"{r.mode}: {v}" if len(chosen) > 1 and field != "**Verdict**" else v)
        return "<br>".join(dict.fromkeys(vals)).replace("|", "/")

    out = []
    in_record = False
    for line in text.splitlines():
        if line.startswith("| Field | Linux | Windows | macOS |"):
            in_record = True
        elif in_record and not line.startswith("|"):
            in_record = False
        if in_record and line.startswith("| ") and not line.startswith(("| Field", "|---")):
            parts = cells(line)
            for i, os_name in enumerate(("linux", "windows", "macos"), start=1):
                value = per_os(os_name, parts[0])
                if value and parts[i] in ("", "PASS / FAIL / WAIVED"):
                    parts[i] = value
            line = join(parts)
        out.append(line)
    return "\n".join(out) + "\n"


def fill_decision(text: str, info: dict, runs: list[Run]) -> str:
    gates = info["gates"]
    covered_os = {r.os for r in runs}
    out = []
    in_table = False
    for line in text.splitlines():
        if line.startswith("| Gate | Status |"):
            in_table = True
        elif in_table and not line.startswith("|"):
            in_table = False
        if in_table and re.match(r"^\| \d", line):
            parts = cells(line)
            nums = re.findall(r"\d+", parts[0].split(" ")[0])
            if "–" in parts[0].split(" ")[0]:
                nums = [str(n) for n in range(int(nums[0]), int(nums[1]) + 1)]
            fail = sum(gates.get(n, {}).get("fail", 0) for n in nums)
            open_ = sum(gates.get(n, {}).get("open", 0) for n in nums)
            if fail:
                parts[1] = f"❌ {fail} failed"
            elif open_:
                parts[1] = f"🟡 {open_} open"
            elif nums and all(n in gates for n in nums):
                parts[1] = "✅"
            line = join(parts)
        out.append(line)
    text = "\n".join(out) + "\n"

    stats = info["stats"]
    blockers_open = len(re.findall(r"^\| B\d+ \|.*\| Open \|$", text, flags=re.M))
    missing = [OS_NAMES[o] for o in OS_NAMES if o not in covered_os]
    if stats["fail"]:
        decision = f"NOT APPROVED — {stats['fail']} automated cell(s) failed"
    elif stats["open"] or stats["skip"] or blockers_open or missing:
        reasons = []
        if missing:
            reasons.append("no run from " + ", ".join(missing))
        if stats["open"] + stats["skip"]:
            reasons.append(f"{stats['open'] + stats['skip']} cell(s) still open")
        if blockers_open:
            reasons.append(f"{blockers_open} blocker(s) open")
        decision = "NOT APPROVED — " + "; ".join(reasons)
    else:
        decision = "READY FOR SIGN-OFF (every cell checked; a person approves)"
    return text.replace("**BETA RELEASE: APPROVED / NOT APPROVED**", f"**BETA RELEASE: {decision}**")


def summary(runs: list[Run], info: dict) -> str:
    s = info["stats"]
    lines = [
        "> **Filled by `ci/vieww checklist`** on "
        + _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        f"> from {len(runs)} run(s): " + ", ".join(f"`{r.path.name}` ({r.label})" for r in runs) + ".",
        f"> ✅ {s['pass']} passed · ❌ {s['fail']} failed · ⚠️ {s['warn']} software GPU · "
        f"⏭️ {s['skip']} skipped · ☐ {s['open']} still open (manual, or no run for that column).",
        "> Only automated cells are filled. Every ☐ left is for a person.",
        "",
    ]
    return "\n".join(lines)


def appendix(runs: list[Run]) -> str:
    out = ["", "---", "", "## Appendix: automated results per run", ""]
    for r in runs:
        out += [f"### `{r.path.name}` ({r.label})", "", "| Row | Check | Verdict | Evidence |", "|---|---|---|---|"]
        for row_id, (what, verdict, ev) in r.rows.items():
            out.append(join([row_id, what, verdict.replace("|", "/"), f"`{ev}`" if ev else ""]))
        out.append("")
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("runs", nargs="+", type=pathlib.Path, help="ci/vieww gate output folders")
    parser.add_argument("-o", "--output", type=pathlib.Path, default=pathlib.Path("CHECKLIST.md"))
    parser.add_argument("--template", type=pathlib.Path, default=TEMPLATE)
    args = parser.parse_args()

    runs = []
    for path in args.runs:
        if not (path / "rows.tsv").is_file():
            print(f"checklist: {path} is not a gate run (no rows.tsv)", file=sys.stderr)
            return 2
        runs.append(Run(path.resolve()))
    cross_run_rows(runs)

    text = args.template.read_text(encoding="utf-8")
    text, info = fill(text, runs)
    text = fill_header(text, runs)
    text = fill_platform_record(text, runs)
    text = fill_decision(text, info, runs)
    title, _, rest = text.partition("\n")
    text = title + "\n\n" + summary(runs, info) + rest + appendix(runs)
    args.output.write_text(text, encoding="utf-8")
    s = info["stats"]
    print(f"wrote {args.output}: {s['pass']} passed, {s['fail']} failed, {s['open']} open")
    return 1 if s["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
