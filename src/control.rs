//! Running the controls of a manifest.

use std::fs;
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::manifest::{self, Control};
use crate::runner;
use crate::scratch::{self, WorkDir};
use crate::verdict::{self, Outcome};

const TAIL_LINES: usize = 6;

#[derive(Debug, Serialize)]
pub struct ControlResult {
    pub id: String,
    pub check: String,
    pub status: &'static str,
    pub reason: String,
    pub baseline: String,
    pub planted_exit: Option<i32>,
    pub output_tail: String,
    pub seconds: f64,
}

pub struct Report {
    pub results: Vec<ControlResult>,
    pub tracked_changes: bool,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|r| r.status == "PASS").count()
    }
    pub fn failed(&self) -> usize {
        self.results.len() - self.passed()
    }
    pub fn exit_code(&self) -> u8 {
        verdict::overall(self.passed(), self.failed())
    }
}

/// Load the manifest and run its controls. `Err` is "could not run" (exit 2).
pub fn run_manifest(path: &Path, only: &[String]) -> Result<Report, String> {
    let mut controls = manifest::load(path)?;
    if !only.is_empty() {
        let missing: Vec<&str> = only
            .iter()
            .filter(|o| !controls.iter().any(|c| &c.id == *o))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "--only names no such control: {}",
                missing.join(", ")
            ));
        }
        controls.retain(|c| only.contains(&c.id));
    }
    if controls.is_empty() {
        return Ok(Report {
            results: Vec::new(),
            tracked_changes: false,
        });
    }
    let manifest_abs =
        fs::canonicalize(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    let root = manifest_abs
        .parent()
        .ok_or("the manifest has no parent directory")?;
    let work = WorkDir::new().map_err(|e| format!("cannot create a temp dir: {e}"))?;
    let head = work.path().join("head");
    scratch::export_head(root, &head)?;
    let tracked_changes = scratch::tracked_changes(root);
    let mut results = Vec::new();
    for (i, c) in controls.iter().enumerate() {
        results.push(run_control(c, root, &head, work.path(), i));
        if runner::interrupted() {
            return Err("interrupted by a signal; nothing is reported".into());
        }
    }
    Ok(Report {
        results,
        tracked_changes,
    })
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

fn baseline_label(o: &Outcome) -> String {
    match o {
        Outcome::Exited(0) => "green".into(),
        Outcome::Exited(c) => format!("red (exit {c})"),
        Outcome::Signaled(s) => format!("red (signal {s})"),
        Outcome::TimedOut => "timeout".into(),
        Outcome::NotStarted(_) => "could not start".into(),
    }
}

fn run_control(c: &Control, root: &Path, head: &Path, work: &Path, i: usize) -> ControlResult {
    let started = Instant::now();
    let mut res = ControlResult {
        id: c.id.clone(),
        check: c.check.clone(),
        status: "FAIL",
        reason: String::new(),
        baseline: if c.baseline { "not run" } else { "skipped" }.into(),
        planted_exit: None,
        output_tail: String::new(),
        seconds: 0.0,
    };
    let scratch_dir = work.join(format!("run-{i}"));
    let log = work.join(format!("run-{i}.log"));
    if let Err(reason) = attempt(c, root, head, &scratch_dir, &log, &mut res) {
        res.reason = reason;
    } else {
        res.status = "PASS";
    }
    let _ = scratch::force_remove(&scratch_dir);
    let _ = fs::remove_file(&log);
    res.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    res
}

/// Ok(()) means PASS, with `res.reason` set; Err carries the FAIL reason.
fn attempt(
    c: &Control,
    root: &Path,
    head: &Path,
    scratch_dir: &Path,
    log: &Path,
    res: &mut ControlResult,
) -> Result<(), String> {
    let plant = scratch::read_plant(root, &c.plant)?;
    verdict::check_plant(&plant.state, &c.plant)?;

    let fresh = || -> Result<(), String> {
        scratch::force_remove(scratch_dir)
            .map_err(|e| format!("cannot reset the scratch copy: {e}"))?;
        if c.shallow_git {
            scratch::shallow_checkout(root, scratch_dir)
        } else {
            scratch::copy_tree(head, scratch_dir).map_err(|e| format!("cannot copy HEAD: {e}"))
        }
    };

    if c.baseline {
        fresh()?;
        let (outcome, out) = runner::run(&c.run, scratch_dir, &c.env, c.timeout, log);
        if runner::interrupted() {
            return Err("interrupted".into());
        }
        res.baseline = baseline_label(&outcome);
        if let Err(reason) = verdict::judge_baseline(&outcome, c.timeout) {
            res.output_tail = tail(&out);
            return Err(reason);
        }
    }
    // A fresh copy for the plant: the baseline may have written caches or reports.
    fresh()?;
    scratch::apply_plant(scratch_dir, &plant).map_err(|e| format!("cannot apply plant: {e}"))?;

    let (outcome, out) = runner::run(&c.run, scratch_dir, &c.env, c.timeout, log);
    res.planted_exit = match outcome {
        Outcome::Exited(code) => Some(code),
        _ => None,
    };
    res.output_tail = tail(&out);
    res.reason = verdict::judge_planted(&outcome, c.timeout, c.regex.as_ref(), &out)?;
    Ok(())
}
