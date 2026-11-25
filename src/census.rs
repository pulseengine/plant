//! `plant census`: which required checks has a planted input turned red?
//!
//! A pure function from files to `plant.census.v1` (docs/plan/v0.4.0.md, bet
//! B). The input directory holds one `<owner>/<repo>/` directory per
//! repository:
//!
//! - `repo.toml`: `default_branch` and `required` (the required contexts);
//! - `.github/workflows/*.yml`: the workflows that produce the contexts;
//! - `ci-controls.toml` (optional): the repository's manifest;
//! - `report.json` (optional): a `plant.report.v1` run report with a `run`
//!   block added by whoever collected it: `branch`, `date` (YYYY-MM-DD) and
//!   `plant_verified` (the plant binary was a release checked by cosign and
//!   sha256).
//!
//! Each required context is put in one class:
//! - `aggregate`: its job only collects the results of the jobs it needs, so
//!   no planted input can be bound to it;
//! - `covered`: a control names it and meets all five conditions (bound, red,
//!   green baseline, fresh, verified);
//! - `declared-not-bound`: a control names it, but none meets them all; the
//!   reasons say which failed;
//! - `uncovered`: no control names it.
//!
//! pulseengine/plant runs plant built from the commit under test, so it is
//! listed under `excluded` and counts in no total.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use yaml_rust2::{Yaml, YamlLoader};

use crate::manifest::{self, Control};

pub const SCHEMA: &str = "plant.census.v1";
pub const REPORT_SCHEMA: &str = "plant.report.v1";
const SELF_REPO: &str = "pulseengine/plant";

#[derive(Debug, Serialize)]
pub struct Census {
    pub schema: &'static str,
    pub month: String,
    pub repos: Vec<RepoCensus>,
    pub excluded: Vec<Excluded>,
    pub totals: Counts,
}

#[derive(Debug, Serialize)]
pub struct Excluded {
    pub repo: String,
    pub reason: &'static str,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Counts {
    pub required: usize,
    pub covered: usize,
    pub declared_not_bound: usize,
    pub aggregate: usize,
    pub uncovered: usize,
}

#[derive(Debug, Serialize)]
pub struct RepoCensus {
    pub repo: String,
    pub counts: Counts,
    pub contexts: Vec<ContextCensus>,
}

#[derive(Debug, Serialize)]
pub struct ContextCensus {
    pub context: String,
    pub class: &'static str,
    pub controls: Vec<String>,
    pub reasons: Vec<String>,
}

/// One job as the census sees it: whether it only aggregates, and every
/// trimmed command line of its `run` steps.
#[derive(Debug, Default)]
struct Job {
    aggregate: bool,
    lines: BTreeSet<String>,
}

struct RunMeta {
    branch: String,
    date: String,
    verified: bool,
}

pub fn run(dir: &Path, month: &str) -> Result<Census, String> {
    if !valid_month(month) {
        return Err(format!("--month {month:?} is not YYYY-MM"));
    }
    let mut repos = Vec::new();
    let mut excluded = Vec::new();
    for owner in sorted_dirs(dir)? {
        for repo in sorted_dirs(&owner)? {
            let name = format!(
                "{}/{}",
                owner.file_name().unwrap().to_string_lossy(),
                repo.file_name().unwrap().to_string_lossy()
            );
            if name == SELF_REPO {
                excluded.push(Excluded {
                    repo: name,
                    reason: "runs plant built from the commit under test, not a verified release",
                });
                continue;
            }
            repos.push(census_repo(&repo, &name, month)?);
        }
    }
    let mut totals = Counts::default();
    for r in &repos {
        totals.required += r.counts.required;
        totals.covered += r.counts.covered;
        totals.declared_not_bound += r.counts.declared_not_bound;
        totals.aggregate += r.counts.aggregate;
        totals.uncovered += r.counts.uncovered;
    }
    Ok(Census {
        schema: SCHEMA,
        month: month.to_string(),
        repos,
        excluded,
        totals,
    })
}

fn valid_month(m: &str) -> bool {
    let b = m.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().chain(&b[5..]).all(u8::is_ascii_digit)
        && matches!(
            &m[5..],
            "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
        )
}

fn sorted_dirs(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    v.sort();
    Ok(v)
}

fn census_repo(dir: &Path, name: &str, month: &str) -> Result<RepoCensus, String> {
    let (default_branch, required) = read_repo_toml(&dir.join("repo.toml"), name)?;
    let jobs = read_jobs(&dir.join(".github/workflows"), name)?;
    let controls = match fs::read_to_string(dir.join("ci-controls.toml")) {
        Ok(t) => manifest::parse(&t).map_err(|e| format!("{name}/ci-controls.toml: {e}"))?,
        Err(_) => Vec::new(),
    };
    let report = read_report(&dir.join("report.json"), name)?;

    // Reasons that apply to every control of this repository.
    let mut report_reasons = Vec::new();
    let results: BTreeMap<String, Value> = match &report {
        None => {
            report_reasons.push("no report.json".to_string());
            BTreeMap::new()
        }
        Some((meta, results)) => {
            if meta.branch != default_branch {
                report_reasons.push(format!(
                    "report is from branch {:?}, not the default branch {default_branch:?}",
                    meta.branch
                ));
            }
            if !meta.date.starts_with(&format!("{month}-")) {
                report_reasons.push(format!("report dated {} is not in {month}", meta.date));
            }
            if !meta.verified {
                report_reasons.push("report is not from a verified plant release".to_string());
            }
            results.clone()
        }
    };

    let mut counts = Counts::default();
    let mut contexts = Vec::new();
    for ctx in required {
        counts.required += 1;
        let producing: Vec<&Job> = jobs
            .get(&ctx)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        let naming: Vec<&Control> = controls.iter().filter(|c| c.check == ctx).collect();
        let ids = naming.iter().map(|c| c.id.clone()).collect();
        let (class, reasons) = if !producing.is_empty() && producing.iter().all(|j| j.aggregate) {
            (
                "aggregate",
                vec!["its job only collects the results of the jobs it needs".to_string()],
            )
        } else if naming.is_empty() {
            ("uncovered", Vec::new())
        } else {
            let mut all = Vec::new();
            let mut ok = false;
            for c in &naming {
                let r = control_reasons(c, &producing, &results, &report_reasons);
                if r.is_empty() {
                    ok = true;
                    break;
                }
                all.extend(r.into_iter().map(|m| format!("{}: {m}", c.id)));
            }
            if ok {
                ("covered", Vec::new())
            } else {
                ("declared-not-bound", all)
            }
        };
        match class {
            "aggregate" => counts.aggregate += 1,
            "covered" => counts.covered += 1,
            "declared-not-bound" => counts.declared_not_bound += 1,
            _ => counts.uncovered += 1,
        }
        contexts.push(ContextCensus {
            context: ctx,
            class,
            controls: ids,
            reasons,
        });
    }
    Ok(RepoCensus {
        repo: name.to_string(),
        counts,
        contexts,
    })
}

/// Every reason this control does not cover its context; empty means covered.
fn control_reasons(
    c: &Control,
    producing: &[&Job],
    results: &BTreeMap<String, Value>,
    report_reasons: &[String],
) -> Vec<String> {
    let mut r = Vec::new();
    if producing.is_empty() {
        r.push(format!("no job is named {:?}", c.check));
    } else if !producing.iter().any(|j| j.lines.contains(c.run.trim())) {
        r.push("run is not a whole command line of a step in the job".to_string());
    }
    if c.regex.is_none() {
        r.push("no stderr_or_stdout_matches regex".to_string());
    }
    if !c.baseline {
        r.push("baseline = false".to_string());
    }
    r.extend(report_reasons.iter().cloned());
    if report_reasons.iter().any(|m| m == "no report.json") {
        return r;
    }
    match results.get(&c.id) {
        None => r.push("the report has no result for this control".to_string()),
        Some(v) => {
            if v["status"] != "PASS" {
                r.push(format!("report status is {}", v["status"]));
            }
            if v["baseline"] != "green" {
                r.push(format!("report baseline is {}", v["baseline"]));
            }
            match v["planted_exit"].as_i64() {
                Some(0) => r.push("the planted run exited 0".to_string()),
                None => r.push("the planted run has no exit code".to_string()),
                Some(_) => {}
            }
        }
    }
    r
}

fn read_repo_toml(path: &Path, name: &str) -> Result<(String, Vec<String>), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{name}/repo.toml: {e}"))?;
    let t: toml::Table = text.parse().map_err(|e| format!("{name}/repo.toml: {e}"))?;
    for k in t.keys() {
        if k != "default_branch" && k != "required" {
            return Err(format!("{name}/repo.toml: unknown key {k:?}"));
        }
    }
    let branch = t
        .get("default_branch")
        .and_then(|v| v.as_str())
        .ok_or(format!("{name}/repo.toml: default_branch must be a string"))?
        .to_string();
    let required: Vec<String> = t
        .get("required")
        .and_then(|v| v.as_array())
        .ok_or(format!(
            "{name}/repo.toml: required must be a list of strings"
        ))?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect::<Option<_>>()
        .ok_or(format!(
            "{name}/repo.toml: required must be a list of strings"
        ))?;
    let unique: BTreeSet<&String> = required.iter().collect();
    if unique.len() != required.len() {
        return Err(format!(
            "{name}/repo.toml: a required context is listed twice"
        ));
    }
    Ok((branch, required))
}

/// Jobs by the context they produce: the job's `name`, or its id when it has
/// none. A name built from an expression (a matrix) is not resolved, so such
/// a job produces no context the census can match.
fn read_jobs(dir: &Path, name: &str) -> Result<BTreeMap<String, Vec<Job>>, String> {
    let mut out: BTreeMap<String, Vec<Job>> = BTreeMap::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return Ok(out);
    };
    let mut files: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        let rel = format!(
            "{name}/.github/workflows/{}",
            f.file_name().unwrap().to_string_lossy()
        );
        let text = fs::read_to_string(&f).map_err(|e| format!("{rel}: {e}"))?;
        let docs = YamlLoader::load_from_str(&text).map_err(|e| format!("{rel}: {e}"))?;
        let Some(jobs) = docs.first().and_then(|d| d["jobs"].as_hash()) else {
            continue;
        };
        for (id, job) in jobs {
            let id = id.as_str().unwrap_or_default();
            let ctx = match job["name"].as_str() {
                Some(n) if n.contains("${{") => continue,
                Some(n) => n.to_string(),
                None => id.to_string(),
            };
            out.entry(ctx).or_default().push(job_of(job));
        }
    }
    Ok(out)
}

fn job_of(job: &Yaml) -> Job {
    let has_needs = match &job["needs"] {
        Yaml::String(_) => true,
        Yaml::Array(a) => !a.is_empty(),
        _ => false,
    };
    let steps = job["steps"].as_vec().cloned().unwrap_or_default();
    let mut lines = BTreeSet::new();
    let mut only_needs = true;
    for s in &steps {
        if !matches!(s["uses"], Yaml::BadValue) {
            only_needs = false;
        }
        if let Some(run) = s["run"].as_str() {
            if !run.contains("needs.") {
                only_needs = false;
            }
            for l in run.lines() {
                let l = l.trim();
                if !l.is_empty() {
                    lines.insert(l.to_string());
                }
            }
        }
    }
    Job {
        aggregate: has_needs && only_needs,
        lines,
    }
}

type Report = (RunMeta, BTreeMap<String, Value>);

fn read_report(path: &Path, name: &str) -> Result<Option<Report>, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}/report.json: {e}"))?;
    if v["schema"] != REPORT_SCHEMA {
        return Err(format!("{name}/report.json: schema is not {REPORT_SCHEMA}"));
    }
    let run = &v["run"];
    let meta = RunMeta {
        branch: run["branch"]
            .as_str()
            .ok_or(format!("{name}/report.json: run.branch must be a string"))?
            .to_string(),
        date: run["date"]
            .as_str()
            .ok_or(format!("{name}/report.json: run.date must be a string"))?
            .to_string(),
        verified: run["plant_verified"].as_bool().ok_or(format!(
            "{name}/report.json: run.plant_verified must be true or false"
        ))?,
    };
    let mut results = BTreeMap::new();
    for c in v["controls"].as_array().cloned().unwrap_or_default() {
        if let Some(id) = c["id"].as_str() {
            results.insert(id.to_string(), c.clone());
        }
    }
    Ok(Some((meta, results)))
}

pub fn json(c: &Census) -> String {
    serde_json::to_string_pretty(c).expect("serializable") + "\n"
}

pub fn text(c: &Census) -> String {
    let mut s = String::new();
    for r in &c.repos {
        let k = &r.counts;
        s.push_str(&format!(
            "{}: {} of {} required contexts covered ({} declared-not-bound, {} aggregate, {} uncovered)\n",
            r.repo, k.covered, k.required, k.declared_not_bound, k.aggregate, k.uncovered
        ));
        for ctx in &r.contexts {
            if ctx.class == "covered" || ctx.class == "uncovered" {
                continue;
            }
            s.push_str(&format!("  {} [{}]\n", ctx.context, ctx.class));
            for m in &ctx.reasons {
                s.push_str(&format!("    {m}\n"));
            }
        }
    }
    for e in &c.excluded {
        s.push_str(&format!("{}: excluded, {}\n", e.repo, e.reason));
    }
    let k = &c.totals;
    s.push_str(&format!(
        "census {}: {} of {} required contexts covered in {} repositories\n",
        c.month,
        k.covered,
        k.required,
        c.repos.len()
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // rivet: verifies REQ-016
    #[test]
    fn a_job_that_only_reads_needs_is_an_aggregate() {
        let y = &YamlLoader::load_from_str(
            "needs: [a, b]\nsteps:\n  - run: test \"${{ needs.a.result }}\" = success\n",
        )
        .unwrap()[0];
        assert!(job_of(y).aggregate);
        let y = &YamlLoader::load_from_str(
            "needs: [a]\nsteps:\n  - uses: actions/checkout@v4\n  - run: cargo test\n",
        )
        .unwrap()[0];
        assert!(!job_of(y).aggregate);
        assert!(job_of(y).lines.contains("cargo test"));
    }

    #[test]
    fn month_must_be_yyyy_mm() {
        assert!(valid_month("2026-11"));
        for m in ["2026-13", "2026-1", "26-11", "2026/11", "2026-00"] {
            assert!(!valid_month(m), "{m}");
        }
    }
}
