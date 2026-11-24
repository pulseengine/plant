//! plant: prove your CI checks can fail.
//!
//! Work in progress. See docs/DESIGN.md for the target behaviour.

mod control;
mod docs;
mod guard;
mod manifest;
mod northstar;
mod report;
mod runner;
mod scratch;
mod verdict;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "plant",
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
    let cli = Cli::parse();
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
