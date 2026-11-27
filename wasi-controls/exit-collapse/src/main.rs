//! plant: prove your CI checks can fail.
//!
//! Work in progress. See docs/DESIGN.md for the target behaviour.

mod census;
#[cfg(not(target_os = "wasi"))]
mod control;
mod docs;
mod guard;
#[cfg_attr(target_os = "wasi", allow(dead_code))]
mod manifest;
mod northstar;
#[cfg(not(target_os = "wasi"))]
mod report;
#[cfg(not(target_os = "wasi"))]
mod runner;
#[cfg(not(target_os = "wasi"))]
mod scratch;
#[cfg(not(target_os = "wasi"))]
mod verdict;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "plant",
    bin_name = "plant",
    version,
    about = "Prove your CI checks can fail: run each one against a planted bad input",
    args_conflicts_with_subcommands = true,
    after_help = docs::EXIT_CODES
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,

    /// The manifest to run
    #[arg(default_value = "ci-controls.toml")]
    manifest: PathBuf,

    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run only the control with this id (repeatable)
    #[arg(long, value_name = "ID")]
    only: Vec<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show embedded documentation; without a topic, list the topics
    Docs { topic: Option<String> },
    /// Write the North Star indicator input (north-star-input.<Ox>.json) from a facts file
    NorthStar {
        /// The facts file, e.g. north-star/O3.toml
        input: PathBuf,
        /// The release version the input belongs to, e.g. v0.4.0
        #[arg(long)]
        version: String,
        /// RFC 3339 timestamp to record (default: now, UTC)
        #[arg(long)]
        generated_at: Option<String>,
    },
    /// Count the required checks a planted input turned red, per repository
    Census {
        /// Directory holding one OWNER/REPO directory per repository
        dir: PathBuf,
        /// The layer month the reports must be from, e.g. 2026-11
        #[arg(long)]
        month: String,
        /// Output format
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Check that no workflow can route a fork pull request to a self-hosted runner
    Guard {
        /// Repository root holding .github/workflows
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Config file (default: DIR/plant-guard.toml when present)
        #[arg(long)]
        config: Option<PathBuf>,
        /// Output format
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

fn main() -> ExitCode {
    let code = plant_main();
    code
}

/// On wasm32-wasip2, both returning an ExitCode and std::process::exit map
/// every nonzero code to 1 (measured, Rust 1.100, wasmtime 50.0.2), so a
/// "could not run" (2) would read as "a control failed" (1). wasi:cli/exit's
/// exit-with-code keeps it. wasmtime 50.0.2 ran it with and without
/// `-S cli-exit-with-code=y`; CI passes the flag.
#[cfg(target_os = "wasi")]
fn wasi_exit(code: ExitCode) {
    if code != ExitCode::SUCCESS {
        let n = if code == ExitCode::from(1) { 1 } else { 2 };
        wasip2::cli::exit::exit_with_code(n);
    }
}

fn plant_main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 2 } else { 0 });
        }
    };
    if let Some(Cmd::NorthStar {
        input,
        version,
        generated_at,
    }) = &cli.command
    {
        return match northstar::run(input, version, generated_at.as_deref()) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("plant north-star: {e}");
                ExitCode::from(2)
            }
        };
    }
    if let Some(Cmd::Census { dir, month, format }) = &cli.command {
        return match census::run(dir, month) {
            Err(e) => {
                eprintln!("plant census: {e}");
                ExitCode::from(2)
            }
            Ok(c) if c.repos.is_empty() => {
                eprintln!("plant census: no repository found; nothing compared");
                ExitCode::from(1)
            }
            Ok(c) => {
                match format {
                    Format::Text => print!("{}", census::text(&c)),
                    Format::Json => print!("{}", census::json(&c)),
                }
                ExitCode::SUCCESS
            }
        };
    }
    if let Some(Cmd::Guard {
        dir,
        config,
        format,
    }) = &cli.command
    {
        let r = guard::load_config(dir, config.as_deref()).and_then(|c| guard::run(dir, &c));
        return match r {
            Err(e) => {
                eprintln!("plant guard: {e}");
                ExitCode::from(2)
            }
            Ok(r) => {
                match format {
                    Format::Text => println!("{}", guard::text(&r)),
                    Format::Json => println!("{}", guard::json(&r)),
                }
                ExitCode::from(r.exit_code())
            }
        };
    }
    if let Some(Cmd::Docs { topic }) = cli.command {
        return match topic {
            None => {
                print!("{}", docs::list());
                ExitCode::SUCCESS
            }
            Some(t) => match docs::topic(&t) {
                Some(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("plant: no docs topic {t:?}\n{}", docs::list());
                    ExitCode::from(2)
                }
            },
        };
    }
    run_controls(&cli)
}

/// Running controls spawns a shell and git, which a WASI component cannot do
/// on its own; the component build carries guard, census, docs and
/// north-star only (REQ-018).
#[cfg(target_os = "wasi")]
fn run_controls(_cli: &Cli) -> ExitCode {
    eprintln!(
        "plant: running controls needs a host shell and git; this WASI build runs only \
         guard, census, docs and north-star"
    );
    ExitCode::from(2)
}

#[cfg(not(target_os = "wasi"))]
fn run_controls(cli: &Cli) -> ExitCode {
    runner::install_signal_handlers();
    match control::run_manifest(&cli.manifest, &cli.only) {
        Err(e) => {
            eprintln!("plant: {e}");
            ExitCode::from(2)
        }
        Ok(report) => {
            if report.tracked_changes {
                eprintln!(
                    "plant: note: tracked files have uncommitted changes; the scratch copy \
                     is HEAD, so those edits are not under test"
                );
            }
            for f in &report.bot_visible {
                eprintln!(
                    "plant: warning: {f} is named like a manifest or lockfile; dependency \
                     bots read it and may open a PR that weakens the plant: store it as \
                     {f}.planted"
                );
            }
            match cli.format {
                Format::Text => println!("{}", report::text(&report)),
                Format::Json => println!("{}", report::json(&report)),
            }
            ExitCode::from(report.exit_code())
        }
    }
}
