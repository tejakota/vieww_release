//! `saygen` — the Say compiler's command line. Reads a `.say` file, writes
//! the generated Rust to stdout. It is how the fixtures in `tests/fixtures`
//! were produced, and how a `.say` file can be inspected without the studio.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: saygen <file.say>");
        return ExitCode::FAILURE;
    }
    let path = &args[1];
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("could not read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    match vieww_say_codegen::compile(&file_name, &source) {
        Ok(generated) => {
            print!("{}", generated.rust);
            ExitCode::SUCCESS
        }
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprintln!("{diagnostic}");
            }
            ExitCode::FAILURE
        }
    }
}
