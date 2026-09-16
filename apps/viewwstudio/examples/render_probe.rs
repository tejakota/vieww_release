//! Render one file from a real workspace and print what the pipeline did.
use std::path::PathBuf;
use vieww_render::FrameDriver;
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::{Shell, Studio, Workspace, WINDOW};

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("workspace"));
    let file = args.next().expect("file");
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let target = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug"));
    let studio = Studio::with_workspace(&runtime, Workspace::open(&root))
        .with_toolchain(Toolchain::discover(&target), Session::new(0x5C_2EE1).ok());
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    studio.open_path(root.join(&file));
    driver.draw_frame();
    studio.render();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while studio.is_compiling() && std::time::Instant::now() < deadline {
        if !studio.poll_compile() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    println!("--- output ---");
    for line in studio.output.peek().iter() {
        println!("{line}");
    }
    println!("--- diagnostics ---");
    for d in studio.diagnostics.peek().iter() {
        println!("{}:{} {} {}", d.file, d.line, d.code, d.message);
    }
}
