//! plant: prove your CI checks can fail.
//!
//! Work in progress. See docs/DESIGN.md for the target behaviour.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => {
            println!("plant {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") | None => {
            println!("plant: prove your CI checks can fail (work in progress)");
            println!("usage: plant [--version] [--help]");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("plant: unknown argument {other:?}; not implemented yet");
            ExitCode::from(2)
        }
    }
}
