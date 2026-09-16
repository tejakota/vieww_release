//! Print the export plan for every format, without running any of it.
use std::path::PathBuf;
use viewwstudio::export::{plan, Format};
use viewwstudio::toolchains::Env;

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("a project root"));
    let name = std::env::args().nth(2).expect("a package name");
    let env = Env::host();
    for format in Format::ALL {
        println!("== {}", format.title());
        match plan(&env, &root, &name, format) {
            Ok(plan) => {
                for step in &plan.steps {
                    println!(
                        "   {} : {} {}",
                        step.spec.label,
                        step.spec.program,
                        step.spec.args.join(" ")
                    );
                }
                println!("   -> {}", plan.artefact.display());
            }
            Err(refusal) => println!("   refused: {refusal}"),
        }
    }
}
