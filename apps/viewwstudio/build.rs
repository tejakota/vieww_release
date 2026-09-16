//! Records the `rustc` this crate was built with, and the commit this is.
//!
//! Two stamps, both of which can only be taken now:
//!
//! * `VIEWWSTUDIO_RUSTC` — the compiler, for the ABI guard, explained below.
//! * `VIEWWSTUDIO_BUILD` — the checkout, for the About box and bug reports.
//!
//! A preview is a `cdylib` handing a `Box<dyn Widget>` across a library
//! boundary. That is only sound if the same compiler built both sides, and the
//! only moment the studio can *know* which compiler built the host is now.
//! `compile::Toolchain::discover` compares this against the `rustc` it finds.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    build_identifier();
    link_arg_rpath();

    let version =
        std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
            .arg("--version")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());

    if let Some(version) = version {
        println!("cargo:rustc-env=VIEWWSTUDIO_RUSTC={version}");
    }

    // The triple this binary is *for*, which under cross-compilation is not
    // the machine building it. `sdk::Manifest::verify` compares an SDK bundle
    // against this, because rlibs are not portable between targets and a
    // bundle assembled for the build host would fail at the first Render with
    // a linker error rather than at startup with a sentence.
    println!(
        "cargo:rustc-env=VIEWWSTUDIO_HOST={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
}

/// Bake an absolute path to the toolchain's `libstd` into every binary.
///
/// # Why a build script has to do what `-C rpath` already claims to
///
/// The studio links `libstd-<hash>.so` dynamically (`.cargo/config.toml`,
/// `prefer-dynamic`) so that a panic inside previewed code is caught by the
/// host's `catch_unwind` rather than aborting the process — see
/// `src/loaded.rs`'s panic section for why both sides have to share one
/// `std`. That makes the binary depend on a file in the toolchain, and the
/// loader has to find it when the studio starts.
///
/// `cargo run` arranges that with `LD_LIBRARY_PATH`. A binary started any
/// other way — a desktop entry, a double click, a bare
/// `./target/debug/viewwstudio` — has only its runpath, and rustc's own
/// `-C rpath` computes a `$ORIGIN`-relative path **for the directory rustc
/// linked into** (`target/debug/deps/`), while cargo copies binaries one
/// level up (`target/debug/`). The relative entry is therefore wrong by one
/// `..` for exactly the binary a person launches, and the studio that
/// supposed to survive a guest panic dies before drawing a window:
///
/// ```text
/// target/debug/viewwstudio: error while loading shared libraries:
/// libstd-<hash>.so: cannot open shared object file
/// ```
///
/// An absolute path has no such ambiguity: it names where the file is on the
/// machine that built, which is the machine that runs a dev checkout. The
/// packaged studio cannot use it — the build machine's toolchain is not on
/// the install machine — which is why `.cargo/config.toml` also carries the
/// two `$ORIGIN`-relative entries for the layouts `packaging/package.sh`
/// installs into, where the depth *is* fixed.
///
/// Unix only, and not because Windows links `std` statically — it does not,
/// since `.cargo/config.toml` sets `prefer-dynamic` there too. Because Windows
/// has **no runpath at all**: `LoadLibrary` reads the executable's own
/// directory, the system directories and `PATH`, never a list baked into the
/// image, so there is nothing for this function to write. Windows gets the same
/// guarantee by having the DLL placed beside the executable —
/// `packaging/package.sh`'s `copy_windows_std`.
///
/// A phone target never runs the studio. The `libstd` check rather than a bare
/// `is_dir` so that an unexpected sysroot layout degrades to the `cargo run`
/// behaviour instead of baking a path to nothing.
fn link_arg_rpath() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if ["windows", "android", "ios"]
        .iter()
        .any(|os| target.contains(os))
    {
        return;
    }
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let Some(sysroot) = std::process::Command::new(&rustc)
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
    else {
        return;
    };
    // The same directory `compile::sysroot_lib` finds at runtime, reached
    // through the triple this build is *for* rather than the host triple
    // `rustc -vV` would report, so a cross-compiled studio gets its own
    // target's libstd rather than the build machine's.
    let lib = std::path::Path::new(&sysroot)
        .join("lib")
        .join("rustlib")
        .join(&target)
        .join("lib");
    let holds_libstd = std::fs::read_dir(&lib).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("libstd-"))
    });
    if !holds_libstd {
        return;
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}

/// Set `VIEWWSTUDIO_BUILD` from the checkout, unless the release workflow
/// already set it.
///
/// # Why the About box used to be useless
///
/// `about::build` reads `option_env!("VIEWWSTUDIO_BUILD")` and falls back to
/// the string "development build". Nothing ever set the variable outside a
/// release workflow that does not exist yet, so **every** build — including one
/// a user is running — reported "development build", and the field in a bug
/// report carried no information at all.
///
/// `git describe --always --dirty --tags` is the answer a checkout can give:
/// `v0.1.0-3-gab12cd`, or a bare short hash in a repository with no tags, with
/// `-dirty` appended when the working tree has uncommitted changes — which is
/// exactly the thing you want to know before trusting a bug report against a
/// commit.
///
/// A source tarball with no `.git`, and a machine with no `git`, still fall
/// back to "development build" through `about`. That is the honest answer
/// there, and it is now the *rare* answer rather than the only one.
///
/// `rerun-if-changed` on `.git/HEAD` and the packed refs keeps the stamp
/// current across a checkout or a commit without rebuilding on every `cargo
/// build`.
fn build_identifier() {
    if std::env::var_os("VIEWWSTUDIO_BUILD").is_some() {
        // The release workflow's value wins: it names the release, and this
        // would overwrite it with a description of the builder's checkout.
        return;
    }
    let Some(root) = git_root() else { return };
    for path in ["HEAD", "packed-refs"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let described = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .current_dir(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()))
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|described| !described.is_empty());
    if let Some(described) = described {
        println!("cargo:rustc-env=VIEWWSTUDIO_BUILD={described}");
    }
}

/// The `.git` directory above the manifest, if this is a checkout at all.
fn git_root() -> Option<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    manifest
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|candidate| candidate.is_dir())
}
