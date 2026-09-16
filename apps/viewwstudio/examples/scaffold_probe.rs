//! Scaffold a project into a directory, so the template can be compiled for real.
fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("a directory"));
    let name = args.next().expect("a crate name");
    let _ = std::fs::remove_dir_all(&root);
    let dep = viewwstudio::scaffold::Dependency::Path(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../.."
    )));
    match viewwstudio::scaffold::create(&root, &name, &dep) {
        Ok(files) => {
            for file in &files {
                println!("{}", file.display());
            }
            println!("{} files", files.len());
        }
        Err(error) => {
            eprintln!("ERROR {error}");
            std::process::exit(1);
        }
    }
}
