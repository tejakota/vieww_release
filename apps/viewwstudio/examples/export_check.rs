//! Studio's export route, end to end: scaffold a project the way New Project
//! does, then build it for every target the way the Export sheet does.
//!
//! ```text
//! cargo run --release -p viewwstudio --example export_check -- OUT [--compile-only] [--only a,b]
//! ```
//!
//! # Why this exists
//!
//! `export.rs` and `harness.rs` are tested as *planners*: which commands an
//! export would run, which files a scaffold writes. Nothing ran those commands.
//! Every Android and iOS line of the templates sits behind
//! `#[cfg(target_os = "android")]` / `"ios"`, so a desktop `cargo test` never
//! compiles them, and a user's first APK export was the first compile they ever
//! got. Two such defects were in the templates when this was written: the Say
//! template called `run_android` with its arguments swapped, and both templates
//! logged through a `log` crate the generated `Cargo.toml` did not depend on.
//!
//! # Two levels, reported separately
//!
//! * **compile** — `cargo check --target <triple>` on both scaffold kinds
//!   (Rust, Say). Needs only `rustup target add`; no NDK, SDK, JDK or Xcode.
//!   Catches every template bug of the kind above, on any machine.
//! * **export** — the Studio's own `export::plan` for each format, run step by
//!   step: `cargo ndk` + Gradle for the APK, `xcodebuild` for iOS, a MinGW
//!   cross-build for Windows. Needs the real toolchains; a format whose
//!   toolchain is missing is reported as `REFUSED(<what>, <install command>)`,
//!   exactly what the Studio would tell a user, and is not a failure.
//!
//! One line per result on stdout — `export-check: <level>/<target> <VERDICT>
//! <detail>` — plus `OUT/results.txt`, and per-step logs under `OUT/logs/`.
//! Exit status 1 if anything FAILED; REFUSED and SKIPPED are not failures.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use viewwstudio::export::{self, Format};
use viewwstudio::scaffold::{self, Dependency, ProjectKind};
use viewwstudio::toolchains::Env;

/// A cross target the compile level checks, with the rustup target that makes
/// it checkable.
const COMPILE_TARGETS: &[(&str, Option<&str>)] = &[
    ("desktop", None),
    ("android", Some("aarch64-linux-android")),
    ("ios", Some("aarch64-apple-ios")),
    ("ios-sim", Some("aarch64-apple-ios-sim")),
    ("windows", Some("x86_64-pc-windows-gnu")),
];

const EXPORT_FORMATS: &[(&str, Format)] = &[
    ("desktop", Format::DesktopBinary),
    ("windows", Format::WindowsExe),
    ("android", Format::AndroidApk),
    ("ios-sim", Format::IosSimulatorApp),
    ("ios-ipa", Format::IosIpa),
];

struct Report {
    out: PathBuf,
    lines: Vec<String>,
    failed: bool,
}

impl Report {
    fn record(&mut self, name: &str, verdict: &str, detail: &str) {
        let line = format!("export-check: {name} {verdict} {detail}");
        println!("{line}");
        if verdict.starts_with("FAIL") {
            self.failed = true;
        }
        self.lines.push(line);
    }
}

/// Run one command with its output in `log`; `Ok(())` on success.
fn run(mut command: Command, log: &Path) -> Result<(), String> {
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::create(log).map_err(|e| e.to_string())?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let status = command
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(err))
        .status()
        .map_err(|e| format!("could not start: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("exit {status}; last lines: {}", last_lines(log, 6)))
    }
}

fn last_lines(log: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("Compiling ") || t.starts_with("Checking ") || t.is_empty())
        })
        .collect();
    lines[lines.len().saturating_sub(n)..].join(" | ")
}

fn installed_targets(dir: &Path) -> Vec<String> {
    // Asked from inside the project, so rustup answers for the toolchain the
    // project's own rust-toolchain.toml selects — the one cargo will use.
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .current_dir(dir)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn scaffold_project(
    out: &Path,
    checkout: &Path,
    kind: ProjectKind,
) -> Result<(PathBuf, String), String> {
    let name = match kind {
        ProjectKind::Rust => "exportcheck-rust",
        ProjectKind::Say => "exportcheck-say",
    };
    let root = out.join(name);
    let _ = std::fs::remove_dir_all(&root);
    scaffold::create_for(&root, name, &Dependency::Path(checkout.to_path_buf()), kind)
        .map_err(|e| e.to_string())?;
    Ok((root, name.to_owned()))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next().map(PathBuf::from) else {
        eprintln!("usage: export_check OUT [--compile-only] [--only desktop,android,...]");
        return ExitCode::from(2);
    };
    let mut compile_only = false;
    let mut only: Option<Vec<String>> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--compile-only" => compile_only = true,
            "--only" => {
                only = args
                    .next()
                    .map(|v| v.split(',').map(str::to_owned).collect())
            }
            other => {
                eprintln!("export_check: unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    let wanted = |name: &str| {
        only.as_ref()
            .is_none_or(|list| list.iter().any(|w| w == name))
    };

    let Some(checkout) = scaffold::checkout_root() else {
        eprintln!(
            "export_check: run this from a vieww checkout (the scaffold needs a path dependency)"
        );
        return ExitCode::from(2);
    };
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("export_check: {e}");
        return ExitCode::from(2);
    }
    // `absolute`, not `canonicalize`: on Windows the latter returns a `\\?\` verbatim
    // path, which breaks every tool it is handed to (cargo, gradle, xcopy).
    let out = std::path::absolute(&out).unwrap_or(out);
    let mut report = Report {
        out: out.clone(),
        lines: Vec::new(),
        failed: false,
    };

    let mut projects = Vec::new();
    for kind in [ProjectKind::Rust, ProjectKind::Say] {
        match scaffold_project(&out, &checkout, kind) {
            Ok(project) => projects.push((kind, project)),
            Err(e) => report.record(&format!("scaffold/{kind:?}"), "FAIL", &e),
        }
    }

    // ── compile: every target, both kinds, no SDKs needed ─────────────────
    // One shared target directory: the two projects build the same dependency
    // graph, and this level never needs artefacts at their default paths.
    let shared_target = out.join("compile-target");
    for (kind, (root, _)) in &projects {
        let installed = installed_targets(root);
        for (target, triple) in COMPILE_TARGETS {
            if !wanted(target) {
                continue;
            }
            let name = format!("compile/{target}-{}", format!("{kind:?}").to_lowercase());
            if let Some(triple) = triple {
                if !installed.iter().any(|t| t == triple) {
                    report.record(
                        &name,
                        "SKIPPED",
                        &format!("rustup target add {triple} (for this project's toolchain)"),
                    );
                    continue;
                }
            }
            let mut cmd = Command::new("cargo");
            cmd.current_dir(root)
                .env("CARGO_TARGET_DIR", &shared_target)
                .arg("check");
            if let Some(triple) = triple {
                cmd.args(["--lib", "--target", triple]);
            } else {
                cmd.arg("--all-targets");
            }
            let log = out
                .join("logs")
                .join(format!("{}.txt", name.replace('/', "-")));
            match run(cmd, &log) {
                Ok(()) => report.record(&name, "PASS", ""),
                Err(e) => report.record(&name, "FAIL", &e),
            }
        }
    }

    // ── export: the Studio's own plans, run for real ──────────────────────
    if !compile_only {
        if let Some((_, (root, name))) = projects.iter().find(|(k, _)| *k == ProjectKind::Rust) {
            let env = Env::host();
            for (label, format) in EXPORT_FORMATS {
                if !wanted(label) {
                    continue;
                }
                let row = format!("export/{label}");
                let plan = match export::plan(&env, root, name, *format) {
                    Ok(plan) => plan,
                    Err(refusal) => {
                        report.record(&row, "REFUSED", &refusal.to_string());
                        continue;
                    }
                };
                let _ = std::fs::remove_file(&plan.artefact);
                let mut ok = true;
                for (i, step) in plan.steps.iter().enumerate() {
                    let spec = &step.spec;
                    let mut cmd = Command::new(&spec.program);
                    cmd.args(&spec.args).current_dir(&spec.dir);
                    for (k, v) in &spec.env {
                        cmd.env(k, v);
                    }
                    let log = out.join("logs").join(format!("export-{label}-{i}.txt"));
                    if let Err(e) = run(cmd, &log) {
                        report.record(
                            &row,
                            "FAIL",
                            &format!("step '{}' ({}): {e}", spec.label, spec.command_line()),
                        );
                        ok = false;
                        break;
                    }
                }
                if ok {
                    let size = std::fs::metadata(&plan.artefact).map(|m| m.len()).ok();
                    match size {
                        Some(bytes) if bytes > 0 || plan.artefact.is_dir() => report.record(
                            &row,
                            "PASS",
                            &format!("{} ({bytes} bytes)", plan.artefact.display()),
                        ),
                        _ => report.record(
                            &row,
                            "FAIL",
                            &format!(
                                "every step succeeded but {} is missing or empty",
                                plan.artefact.display()
                            ),
                        ),
                    }
                }
            }
        }
    }

    let _ = std::fs::write(
        report.out.join("results.txt"),
        report.lines.join("\n") + "\n",
    );
    if report.failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
