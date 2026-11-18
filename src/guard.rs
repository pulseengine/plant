//! `plant guard`: no fork pull request reaches a self-hosted runner.
//!
//! Ported from pulseengine/relay's scripts/check-selfhosted-fork-guard.rs
//! (relay#695, f852816), itself a port of pulseengine/ordeal#322. The decision
//! rules and their 19 tests are relay's; plant adds the per-repo config
//! (`plant-guard.toml`): the hosted fallback image, the provisioned runner
//! labels, and exemptions for workflows a fork cannot trigger.
//!
//! Every line in .github/workflows/*.yml that names `self-hosted` must be one of:
//!
//!   1. a `runs-on:` in exactly the fail-closed same-repo form
//!      `${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('[..."self-hosted"...]') || '<fallback>' }}`
//!      so the self-hosted labels are chosen only for a non-PR event or a PR
//!      whose head repository is this one. A fork, or a deleted fork whose
//!      `head.repo` is null, falls to the hosted runner;
//!   2. a matrix `runner:` value inside a job whose `runs-on:` is exactly the
//!      matrix variant of that form, which also sends a self-hosted matrix leg
//!      to the hosted runner for a foreign head.
//!
//! Every `runs-on:` is allowlisted: a hosted image (ubuntu-*, macos-*,
//! windows-*), the fail-closed form, the matrix form, or a plain
//! `${{ matrix.<id> }}` whose every literal value is a hosted image. GitHub
//! matches labels, so `light` or `[linux, light]` reach the self-hosted pool
//! without the word self-hosted. A runner taken from needs.*.outputs, inputs,
//! vars or github.* is rejected, and so is a block-list or map runs-on.
//!
//! Any self-hosted job in a workflow with a trigger outside TRUSTED is
//! rejected: the form trusts every event that is not `pull_request`. A run
//! that finds no self-hosted line fails: it compared nothing.
//!
//! The parse is line-based and fails closed: a non-comment line containing
//! `self-hosted` that is not an accepted shape is a violation.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const GUARD: &str = "(github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository)";
const MATRIX_GUARD: &str = "(github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository || !contains(matrix.runner, 'self-hosted'))";
/// The hosted image a fork PR falls back to when the config names none.
pub const DEFAULT_FALLBACK: &str = "ubuntu-24.04";
/// `release` is plant's addition to relay's list: only a user with write
/// access can publish a release, so a fork cannot cause the event.
const TRUSTED: &[&str] = &[
    "push",
    "pull_request",
    "schedule",
    "workflow_dispatch",
    "merge_group",
    "release",
];
/// Triggers under which an exempt workflow may select self-hosted runners
/// without the fail-closed form: none of them can be caused by a fork.
const EXEMPTABLE: &[&str] = &["push", "schedule", "workflow_dispatch", "release"];
pub const CONFIG_FILE: &str = "plant-guard.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    fallback: Option<String>,
    labels: Option<Vec<String>>,
    #[serde(default)]
    exempt: Vec<Exempt>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Exempt {
    pub workflow: String,
    pub reason: String,
}

pub struct Config {
    pub fallback: String,
    /// The provisioned runner labels; None when the config declares none.
    pub labels: Option<BTreeSet<String>>,
    pub exempt: Vec<Exempt>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            fallback: DEFAULT_FALLBACK.into(),
            labels: None,
            exempt: vec![],
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let f: ConfigFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let fallback = f.fallback.unwrap_or_else(|| DEFAULT_FALLBACK.into());
        if !is_hosted(&fallback) || fallback.contains('\'') {
            return Err(format!(
                "fallback {fallback:?} is not a GitHub-hosted image name"
            ));
        }
        let labels = match f.labels {
            None => None,
            Some(v) => {
                if v.is_empty() {
                    return Err("labels is empty; omit it or name the provisioned labels".into());
                }
                if let Some(bad) = v
                    .iter()
                    .find(|l| l.trim().is_empty() || l.trim() != l.as_str())
                {
                    return Err(format!(
                        "label {bad:?} is empty or has surrounding whitespace"
                    ));
                }
                Some(v.into_iter().collect())
            }
        };
        for e in &f.exempt {
            if e.workflow.is_empty() || e.workflow.contains('/') {
                return Err(format!(
                    "exempt workflow {:?} must be a file name in .github/workflows",
                    e.workflow
                ));
            }
            if e.reason.trim().is_empty() {
                return Err(format!("exempt workflow {:?} has no reason", e.workflow));
            }
        }
        Ok(Config {
            fallback,
            labels,
            exempt: f.exempt,
        })
    }

    fn form_tail(&self) -> String {
        format!("]') || '{}' }}}}", self.fallback)
    }

    fn matrix_form(&self) -> String {
        format!(
            "${{{{ {MATRIX_GUARD} && matrix.runner || '{}' }}}}",
            self.fallback
        )
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Violation {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    pub reason: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match (self.line, &self.job) {
            (Some(n), Some(j)) => write!(f, "{}:{n} ({j}): {}", self.file, self.reason),
            (Some(n), None) => write!(f, "{}:{n}: {}", self.file, self.reason),
            _ => write!(f, "{}: {}", self.file, self.reason),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Report {
    pub seen: usize,
    pub bad: Vec<Violation>,
}

/// The labels of the fail-closed single-label form, or None when `v` is not
/// that form.
fn runs_on_form_labels(v: &str, cfg: &Config) -> Option<Vec<String>> {
    let rest = v
        .strip_prefix("${{ ")?
        .strip_prefix(GUARD)?
        .strip_prefix(" && fromJSON('[")?;
    let end = rest.find("]')")?;
    let labels = &rest[..end];
    if labels.contains('\'') || rest[end..] != cfg.form_tail() {
        return None;
    }
    let list: Vec<String> = serde_json::from_str(&format!("[{labels}]")).ok()?;
    list.iter().any(|l| l == "self-hosted").then_some(list)
}

/// Strips a trailing YAML comment that is outside quotes.
fn strip_comment(line: &str) -> &str {
    let (mut sq, mut dq) = (false, false);
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !dq => sq = !sq,
            '"' if !sq => dq = !dq,
            '#' if !sq && !dq && (i == 0 || line[..i].ends_with(char::is_whitespace)) => {
                return &line[..i];
            }
            _ => {}
        }
    }
    line
}

/// The key of `${{ matrix.<ident> }}` exactly, else None.
fn plain_matrix_key(v: &str) -> Option<&str> {
    v.strip_prefix("${{ matrix.")
        .and_then(|r| r.strip_suffix(" }}"))
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

/// A GitHub-hosted image name: ubuntu-*, macos-* or windows-*.
fn is_hosted(v: &str) -> bool {
    let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
    ["ubuntu-", "macos-", "windows-"].iter().any(|p| {
        v.strip_prefix(p).is_some_and(|r| {
            !r.is_empty()
                && r.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
    })
}

/// Items of a YAML flow list `[a, "b"]`, else None.
fn flow_items(v: &str) -> Option<Vec<String>> {
    let inner = v.trim().strip_prefix('[')?.strip_suffix(']')?;
    Some(
        inner
            .split(',')
            .map(|i| i.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
            .filter(|i| !i.is_empty())
            .collect(),
    )
}

/// A bare runner label: a plain scalar, not an expression, list or map.
fn bare_label(v: &str) -> Option<String> {
    let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
    (!v.is_empty() && !v.contains(['$', '{', '}', '[', ']', ',', ' ', ':'])).then(|| v.to_string())
}

/// Every literal value written for matrix key `key` in a job's lines: inline
/// (`key: v`, `- key: v`, `key: [a, b]`) or as a block list under `key:`.
fn matrix_values(job_lines: &[&str], key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let head = format!("{key}:");
    for (i, raw) in job_lines.iter().enumerate() {
        let t = strip_comment(raw).trim();
        let t = t.strip_prefix("- ").unwrap_or(t);
        let Some(rest) = t.strip_prefix(&head) else {
            continue;
        };
        let rest = rest.trim();
        if !rest.is_empty() {
            out.push(rest.to_string());
            continue;
        }
        let ind = indent(raw);
        for l in &job_lines[i + 1..] {
            let lt = strip_comment(l).trim();
            if lt.is_empty() {
                continue;
            }
            if indent(l) <= ind && !lt.starts_with("- ") || indent(l) < ind {
                break;
            }
            match lt.strip_prefix("- ") {
                Some(item) if !item.contains(':') => out.push(item.trim().to_string()),
                _ => break,
            }
        }
    }
    out
}

/// A `key: |` / `key: >-` style line, whose following deeper lines are text.
fn is_block_scalar_head(t: &str) -> bool {
    let t = t.trim_start_matches("- ");
    match t.split_once(':') {
        Some((_, v)) => {
            let v = v.trim();
            !v.is_empty() && (v.starts_with('|') || v.starts_with('>')) && v.len() <= 3
        }
        None => false,
    }
}

/// A `name:` line (a workflow, job or step name) is display text: it cannot
/// select a runner, so prose naming self-hosted there is not a site.
fn is_name_line(t: &str) -> bool {
    t.trim_start_matches("- ").starts_with("name:")
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The workflow's trigger names, from a top-level `on:` (block map, flow list
/// or scalar). Returns None when there is no `on:` key.
fn triggers(lines: &[&str]) -> Option<Vec<String>> {
    let i = lines.iter().position(|l| {
        let t = strip_comment(l).trim_end();
        indent(l) == 0 && (t == "on:" || t.starts_with("on: ") || t == "\"on\":" || t == "'on':")
    })?;
    let head = strip_comment(lines[i]).trim_end();
    let inline = head.split_once(':').map(|x| x.1.trim()).unwrap_or("");
    if !inline.is_empty() {
        let inner = inline.trim_start_matches('[').trim_end_matches(']');
        return Some(
            inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        );
    }
    let mut out = Vec::new();
    let mut child: Option<usize> = None;
    for l in &lines[i + 1..] {
        let t = strip_comment(l).trim_end();
        if t.trim().is_empty() {
            continue;
        }
        let ind = indent(l);
        if ind == 0 {
            break;
        }
        let c = *child.get_or_insert(ind);
        if ind == c {
            let key = t.trim().trim_start_matches("- ");
            let key = key.split(':').next().unwrap_or("").trim();
            out.push(key.to_string());
        }
    }
    Some(out)
}

/// Checks one workflow's text. `seen` counts self-hosted selections compared.
/// `exempt` is true when plant-guard.toml exempts this workflow.
pub fn check_workflow(name: &str, text: &str, cfg: &Config, exempt: bool) -> Report {
    let lines: Vec<&str> = text.lines().collect();
    let matrix_form = cfg.matrix_form();
    let mut bad = Vec::new();
    let mut seen = 0;
    let mut job_matrix_ok = false;
    let mut jobs_indent: Option<usize> = None;
    let mut job_indent: Option<usize> = None;
    let mut job = String::from("?");
    let mut job_span = (0usize, 0usize);
    // Indent of an open block scalar (`run: |`, `body: >-`); its body is shell
    // or prose, not workflow structure, and is skipped.
    let mut block: Option<usize> = None;
    let at = |n: usize, job: &str, reason: String| Violation {
        file: name.to_string(),
        line: Some(n + 1),
        job: Some(job.to_string()),
        reason,
    };
    let check_labels = |n: usize, job: &str, labels: &[String], bad: &mut Vec<Violation>| {
        if let Some(set) = &cfg.labels {
            for l in labels.iter().filter(|l| !set.contains(l.as_str())) {
                bad.push(at(
                    n,
                    job,
                    format!("runner label {l:?} is not in {CONFIG_FILE} labels"),
                ));
            }
        }
    };
    for (n, raw) in lines.iter().enumerate() {
        let code = strip_comment(raw);
        let t = code.trim();
        if t.is_empty() {
            continue;
        }
        let ind = indent(raw);
        if let Some(b) = block {
            if ind > b {
                continue;
            }
            block = None;
        }
        if is_block_scalar_head(t) {
            block = Some(ind);
        }
        if ind == 0 {
            jobs_indent = if t == "jobs:" { Some(0) } else { None };
            job_indent = None;
            continue;
        }
        if jobs_indent.is_some() {
            let ji = *job_indent.get_or_insert(ind);
            if ind == ji && t.ends_with(':') {
                job = t.trim_end_matches(':').to_string();
                let len = lines[n + 1..]
                    .iter()
                    .take_while(|l| strip_comment(l).trim().is_empty() || indent(l) > ji)
                    .count();
                job_span = (n + 1, n + 1 + len);
                job_matrix_ok = lines[job_span.0..job_span.1]
                    .iter()
                    .any(|l| strip_comment(l).trim() == format!("runs-on: {matrix_form}"));
                continue;
            }
        }
        // Every runs-on is shape-checked, not only ones that spell
        // `self-hosted`: a runner computed into needs.*.outputs, inputs, vars
        // or github.* names no label here, yet can resolve to self-hosted.
        let mut counted = false;
        if let Some(v) = t.strip_prefix("runs-on:") {
            let v = v.trim();
            let job_lines = &lines[job_span.0..job_span.1];
            let ok = if let Some(labels) = runs_on_form_labels(v, cfg) {
                check_labels(n, &job, &labels, &mut bad);
                true
            } else if v == matrix_form {
                let vals = matrix_values(job_lines, "runner");
                !vals.is_empty()
                    && vals.iter().all(|x| match flow_items(x) {
                        Some(it) if it.iter().any(|i| i == "self-hosted") => {
                            check_labels(n, &job, &it, &mut bad);
                            true
                        }
                        Some(it) => it.iter().all(|i| is_hosted(i)),
                        None => is_hosted(x),
                    })
            } else if let Some(key) = plain_matrix_key(v) {
                let vals = matrix_values(job_lines, key);
                !vals.is_empty()
                    && vals.iter().all(|x| match flow_items(x) {
                        Some(it) => !it.is_empty() && it.iter().all(|i| is_hosted(i)),
                        None => is_hosted(x),
                    })
            } else if is_hosted(v) {
                true
            } else if exempt {
                // An exempt workflow may name provisioned labels directly.
                let labels = match flow_items(v) {
                    Some(it) if !it.is_empty() && it.iter().all(|i| bare_label(i).is_some()) => {
                        Some(it)
                    }
                    Some(_) => None,
                    None => bare_label(v).map(|l| vec![l]),
                };
                match labels {
                    Some(l) => {
                        check_labels(n, &job, &l, &mut bad);
                        seen += 1;
                        counted = true;
                        true
                    }
                    None => false,
                }
            } else {
                false
            };
            if !ok {
                bad.push(at(n, &job, "runs-on is not a hosted image, the fail-closed form, or a matrix whose every runner is a hosted image".into()));
            }
        }
        if !t.contains("self-hosted") || is_name_line(t) {
            continue;
        }
        if !counted {
            seen += 1;
        }
        if t.starts_with("runs-on:") {
            // Judged by the allowlist above.
        } else if t.starts_with("runner:") || t.starts_with("- runner:") {
            if !job_matrix_ok {
                bad.push(at(n, &job, "self-hosted matrix runner in a job whose runs-on is not the fail-closed matrix form".into()));
            }
        } else {
            bad.push(at(
                n,
                &job,
                "self-hosted outside an accepted runs-on/runner shape".into(),
            ));
        }
    }
    let whole = |reason: String| Violation {
        file: name.to_string(),
        line: None,
        job: None,
        reason,
    };
    if seen > 0 || exempt {
        match triggers(&lines) {
            None => bad.push(whole(
                "has self-hosted jobs but no readable top-level `on:`".into(),
            )),
            Some(ev) => {
                let untrusted: Vec<_> = ev
                    .iter()
                    .filter(|e| !TRUSTED.contains(&e.as_str()))
                    .cloned()
                    .collect();
                if seen > 0 && !untrusted.is_empty() {
                    bad.push(whole(format!(
                        "self-hosted jobs under trigger(s) outside the allowlist: {}",
                        untrusted.join(", ")
                    )));
                }
                let reachable: Vec<_> = ev
                    .iter()
                    .filter(|e| !EXEMPTABLE.contains(&e.as_str()))
                    .cloned()
                    .collect();
                if exempt && !reachable.is_empty() {
                    bad.push(whole(format!(
                        "exempt in {CONFIG_FILE}, but trigger(s) {} can be caused by a pull request; an exemption holds only under push, schedule and workflow_dispatch",
                        reachable.join(", ")
                    )));
                }
            }
        }
    }
    Report { seen, bad }
}

pub struct GuardRun {
    pub workflows: usize,
    pub seen: usize,
    pub bad: Vec<Violation>,
    pub fallback: String,
}

impl GuardRun {
    pub fn exit_code(&self) -> u8 {
        if self.seen == 0 || !self.bad.is_empty() {
            1
        } else {
            0
        }
    }
}

/// Reads the config: `explicit` when given (it must exist), else
/// DIR/plant-guard.toml when present, else the defaults.
pub fn load_config(dir: &Path, explicit: Option<&Path>) -> Result<Config, String> {
    let path: PathBuf = match explicit {
        Some(p) => p.to_path_buf(),
        None => {
            let p = dir.join(CONFIG_FILE);
            if !p.exists() {
                return Ok(Config::default());
            }
            p
        }
    };
    let text =
        fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Checks every workflow under DIR/.github/workflows. Err means could not run.
pub fn run(dir: &Path, cfg: &Config) -> Result<GuardRun, String> {
    let wdir = dir.join(".github/workflows");
    let mut entries: Vec<PathBuf> = fs::read_dir(&wdir)
        .map_err(|e| format!("could not read {}: {e}", wdir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("yml" | "yaml")))
        .collect();
    entries.sort();
    let names: BTreeSet<String> = entries
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let exempt: BTreeSet<&str> = cfg.exempt.iter().map(|e| e.workflow.as_str()).collect();
    let (mut seen, mut bad) = (0, Vec::new());
    for e in &cfg.exempt {
        if !names.contains(&e.workflow) {
            bad.push(Violation {
                file: CONFIG_FILE.into(),
                line: None,
                job: None,
                reason: format!(
                    "exempts {:?}, which is not in .github/workflows",
                    e.workflow
                ),
            });
        }
    }
    for p in &entries {
        let text =
            fs::read_to_string(p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let r = check_workflow(&name, &text, cfg, exempt.contains(name.as_str()));
        seen += r.seen;
        bad.extend(r.bad);
    }
    if seen > 0 && cfg.labels.is_none() {
        bad.push(Violation {
            file: CONFIG_FILE.into(),
            line: None,
            job: None,
            reason:
                "workflows select self-hosted runners, but no provisioned `labels` are declared"
                    .into(),
        });
    }
    Ok(GuardRun {
        workflows: entries.len(),
        seen,
        bad,
        fallback: cfg.fallback.clone(),
    })
}

pub fn text(r: &GuardRun) -> String {
    let mut out = String::new();
    for b in &r.bad {
        out.push_str(&format!("FAIL  {b}\n"));
    }
    if r.seen == 0 {
        out.push_str(&format!(
            "FAIL  no self-hosted line found in {} workflow(s): this check compared nothing",
            r.workflows
        ));
    } else if !r.bad.is_empty() {
        out.push_str(&format!(
            "FAIL  {} violation(s) across {} self-hosted line(s)",
            r.bad.len(),
            r.seen
        ));
    } else {
        out.push_str(&format!(
            "self-hosted fork guard: OK ({} self-hosted line(s) in {} workflow(s), all fail-closed)",
            r.seen, r.workflows
        ));
    }
    out
}

pub fn json(r: &GuardRun) -> String {
    serde_json::json!({
        "schema": "plant.guard.v1",
        "ok": r.exit_code() == 0,
        "workflows": r.workflows,
        "self_hosted_lines": r.seen,
        "compared_nothing": r.seen == 0,
        "fallback": r.fallback,
        "violations": r.bad,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            fallback: DEFAULT_FALLBACK.into(),
            labels: Some(
                ["self-hosted", "linux", "x64", "light", "rust-cpu"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ),
            exempt: vec![],
        }
    }
    fn check(name: &str, text: &str) -> Report {
        check_workflow(name, text, &cfg(), false)
    }
    fn good(label: &str) -> String {
        format!("${{{{ {GUARD} && fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"{label}\"]') || '{DEFAULT_FALLBACK}' }}}}")
    }
    fn wf(on: &str, runs_on: &str) -> String {
        format!(
            "name: t\n{on}\njobs:\n  a:\n    runs-on: {runs_on}\n    steps:\n      - run: true\n"
        )
    }
    fn matrix_form() -> String {
        cfg().matrix_form()
    }

    // The 19 decision cases below are relay's (f852816), with `bad` counted.

    // rivet: verifies REQ-011
    #[test]
    fn accepts_the_fail_closed_form() {
        let r = check(
            "t.yml",
            &wf("on:\n  pull_request:\n  push:", &good("light")),
        );
        assert_eq!(
            r,
            Report {
                seen: 1,
                bad: vec![]
            }
        );
    }

    // rivet: verifies REQ-011
    #[test]
    fn rejects_the_fail_open_fork_form() {
        let v = "${{ github.event.pull_request.head.repo.fork && 'ubuntu-latest' || fromJSON('[\"self-hosted\",\"linux\"]') }}";
        let r = check("t.yml", &wf("on: [pull_request]", v));
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-011
    #[test]
    fn rejects_an_unpinned_fallback() {
        let v = format!(
            "${{{{ {GUARD} && fromJSON('[\"self-hosted\",\"light\"]') || 'ubuntu-latest' }}}}"
        );
        let r = check("t.yml", &wf("on: [pull_request]", &v));
        assert_eq!(r.bad.len(), 1, "{r:?}");
        let m = matrix_form().replace(DEFAULT_FALLBACK, "ubuntu-latest");
        let t = format!("name: t\non: [pull_request]\njobs:\n  a:\n    strategy:\n      matrix:\n        include:\n          - runner: [self-hosted, light]\n    runs-on: {m}\n    steps:\n      - run: true\n");
        assert!(!check("t.yml", &t).bad.is_empty());
    }

    // rivet: verifies REQ-011
    #[test]
    fn rejects_a_plain_label_list() {
        let r = check(
            "t.yml",
            &wf("on:\n  schedule:", "[self-hosted, linux, x64, light]"),
        );
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-011
    #[test]
    fn rejects_swapped_branches() {
        let v = format!("${{{{ {GUARD} && 'ubuntu-latest' || fromJSON('[\"self-hosted\"]') }}}}");
        let r = check("t.yml", &wf("on: [pull_request]", &v));
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-011
    #[test]
    fn rejects_untrusted_triggers_even_with_the_form() {
        for on in [
            "on:\n  pull_request_target:",
            "on: [workflow_run]",
            "on:\n  issue_comment:\n    types: [created]",
        ] {
            let r = check("t.yml", &wf(on, &good("light")));
            assert_eq!(r.bad.len(), 1, "{on}: {r:?}");
        }
    }

    // rivet: verifies REQ-011
    #[test]
    fn trigger_parse_ignores_nested_keys() {
        let on = "on:\n  push:\n    branches: [main]\n  pull_request:\n    paths:\n      - 'x'";
        let r = check("t.yml", &wf(on, &good("light")));
        assert_eq!(
            r,
            Report {
                seen: 1,
                bad: vec![]
            }
        );
    }

    // rivet: verifies REQ-011
    #[test]
    fn matrix_runner_needs_the_matrix_form() {
        let body = |ro: &str| {
            format!(
                "on: [pull_request]\njobs:\n  test:\n    strategy:\n      matrix:\n        include:\n          - label: linux\n            runner: [self-hosted, linux, x64, rust-cpu]\n          - label: macos\n            runner: macos-14\n    runs-on: {ro}\n    steps:\n      - run: true\n"
            )
        };
        assert_eq!(check("t.yml", &body(&matrix_form())).bad, vec![]);
        let fail_open = "${{ github.event.pull_request.head.repo.fork && contains(matrix.runner, 'self-hosted') && 'ubuntu-latest' || matrix.runner }}";
        assert_eq!(check("t.yml", &body(fail_open)).bad.len(), 2);
    }

    // rivet: verifies REQ-011
    #[test]
    fn self_hosted_in_an_unknown_shape_fails_closed() {
        let t = "on: [pull_request]\njobs:\n  a:\n    runs-on:\n      - self-hosted\n    steps:\n      - run: true\n";
        assert!(!check("t.yml", t).bad.is_empty());
    }

    // rivet: verifies REQ-011
    #[test]
    fn comments_naming_self_hosted_are_ignored() {
        let t = format!(
            "# runs on self-hosted\n{}",
            wf("on: [push]", &good("rust-cpu"))
        );
        assert_eq!(
            check("t.yml", &t),
            Report {
                seen: 1,
                bad: vec![]
            }
        );
    }

    // rivet: verifies REQ-011
    #[test]
    fn shell_text_in_run_blocks_is_not_structure() {
        let t = format!(
            "on: [push]\njobs:\n  a:\n    runs-on: {}\n    steps:\n      - run: |\n          echo self-hosted fleet\n          kind=\"self-hosted\"\n      - name: x\n        run: true\n",
            good("light")
        );
        assert_eq!(
            check("t.yml", &t),
            Report {
                seen: 1,
                bad: vec![]
            }
        );
    }

    // rivet: verifies REQ-011
    #[test]
    fn a_runs_on_after_a_run_block_is_still_checked() {
        let t = "on: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          echo hi\n  b:\n    runs-on: [self-hosted, light]\n";
        assert_eq!(check("t.yml", t).bad.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn computed_runners_are_rejected() {
        for v in [
            "${{ needs.pick.outputs.runner }}",
            "${{ inputs.runner }}",
            "${{ vars.RUNNER }}",
            "${{ fromJSON(needs.pick.outputs.labels) }}",
            "${{ github.event.inputs.runner || 'ubuntu-latest' }}",
        ] {
            let r = check("t.yml", &wf("on: [workflow_dispatch]", v));
            assert_eq!(r.bad.len(), 1, "{v}: {r:?}");
        }
    }

    // rivet: verifies REQ-011
    #[test]
    fn plain_matrix_and_literal_hosted_runners_pass() {
        for v in ["ubuntu-latest", "macos-14", "windows-2022"] {
            let r = check("t.yml", &wf("on: [pull_request]", v));
            assert_eq!(
                r,
                Report {
                    seen: 0,
                    bad: vec![]
                },
                "{v}"
            );
        }
    }

    // rivet: verifies REQ-011
    #[test]
    fn block_and_map_runs_on_are_rejected() {
        let list = "on: [push]\njobs:\n  a:\n    runs-on:\n      - ${{ needs.x.outputs.r }}\n";
        let map = "on: [push]\njobs:\n  a:\n    runs-on: { group: fleet }\n";
        assert_eq!(check("t.yml", list).bad.len(), 1);
        assert_eq!(check("t.yml", map).bad.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn bare_and_listed_custom_labels_are_rejected() {
        for v in ["light", "[linux, light]", "[light]", "rust-cpu"] {
            let r = check("t.yml", &wf("on: [schedule]", v));
            assert_eq!(r.bad.len(), 1, "{v}: {r:?}");
        }
    }

    // rivet: verifies REQ-011
    #[test]
    fn plain_matrix_ref_needs_every_value_hosted() {
        let job = |axis: &str| {
            format!("on: [push]\njobs:\n  a:\n    strategy:\n      matrix:\n{axis}\n    runs-on: ${{{{ matrix.os }}}}\n")
        };
        assert!(
            check("t.yml", &job("        os: [ubuntu-latest, macos-14]"))
                .bad
                .is_empty()
        );
        assert!(check(
            "t.yml",
            &job("        os:\n          - ubuntu-24.04\n          - windows-2022")
        )
        .bad
        .is_empty());
        assert!(check(
            "t.yml",
            &job("        include:\n          - os: ubuntu-latest\n          - os: macos-14")
        )
        .bad
        .is_empty());
        assert_eq!(
            check("t.yml", &job("        os: [ubuntu-latest, light]"))
                .bad
                .len(),
            1
        );
        assert_eq!(
            check(
                "t.yml",
                &job("        include:\n          - os: ubuntu-latest\n          - os: light")
            )
            .bad
            .len(),
            1
        );
        assert_eq!(
            check("t.yml", &job("        os: ${{ fromJSON(vars.OSES) }}"))
                .bad
                .len(),
            1
        );
        assert_eq!(check("t.yml", &job("        arch: [x64]")).bad.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn matrix_form_rejects_a_custom_label_runner() {
        let job = "on: [pull_request]\njobs:\n  test:\n    strategy:\n      matrix:\n        include:\n          - runner: [linux, light]\n";
        let t = format!("{job}    runs-on: {}\n", matrix_form());
        assert_eq!(check("t.yml", &t).bad.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn a_workflow_without_self_hosted_is_not_counted() {
        let r = check("t.yml", &wf("on: [pull_request_target]", "ubuntu-latest"));
        assert_eq!(
            r,
            Report {
                seen: 0,
                bad: vec![]
            }
        );
    }

    // plant's additions: configured fallback, labels, exemptions.

    // rivet: verifies REQ-011
    #[test]
    fn the_fallback_is_a_per_repo_setting() {
        let c =
            Config::parse("fallback = \"ubuntu-22.04\"\nlabels = [\"self-hosted\", \"light\"]\n")
                .unwrap();
        let v = format!(
            "${{{{ {GUARD} && fromJSON('[\"self-hosted\",\"light\"]') || 'ubuntu-22.04' }}}}"
        );
        assert!(
            check_workflow("t.yml", &wf("on: [pull_request]", &v), &c, false)
                .bad
                .is_empty()
        );
        let pinned_elsewhere = check_workflow(
            "t.yml",
            &wf("on: [pull_request]", &good("light")),
            &c,
            false,
        );
        assert_eq!(pinned_elsewhere.bad.len(), 1, "{pinned_elsewhere:?}");
        assert!(Config::parse("fallback = \"light\"").is_err());
        assert!(Config::parse("fallback = \"ubuntu-latest' || 'light\"").is_err());
    }

    // rivet: verifies REQ-012
    #[test]
    fn a_label_outside_the_declared_set_is_rejected() {
        let r = check("t.yml", &wf("on: [pull_request]", &good("}.$l.q{")));
        assert_eq!(r.bad.len(), 1, "{r:?}");
        assert!(r.bad[0].reason.contains("\"}.$l.q{\""), "{r:?}");
        let r = check("t.yml", &wf("on: [pull_request]", &good("lihgt")));
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-012
    #[test]
    fn a_matrix_runner_label_outside_the_set_is_rejected() {
        let t = format!(
            "on: [pull_request]\njobs:\n  test:\n    strategy:\n      matrix:\n        include:\n          - runner: [self-hosted, linux, gpu]\n    runs-on: {}\n",
            matrix_form()
        );
        let r = check("t.yml", &t);
        assert_eq!(r.bad.len(), 1, "{r:?}");
        assert!(r.bad[0].reason.contains("\"gpu\""));
    }

    // rivet: verifies REQ-012
    #[test]
    fn a_form_whose_label_list_is_not_json_is_rejected() {
        let v = format!(
            "${{{{ {GUARD} && fromJSON('[self-hosted, light]') || '{DEFAULT_FALLBACK}' }}}}"
        );
        assert_eq!(check("t.yml", &wf("on: [push]", &v)).bad.len(), 1);
    }

    // rivet: verifies REQ-012
    #[test]
    fn labels_are_not_checked_when_none_are_declared_but_the_run_fails() {
        let dir = tempdir_with("x.yml", &wf("on: [push]", &good("light")));
        let r = run(dir.path(), &Config::default()).unwrap();
        assert_eq!(r.bad.len(), 1, "{:?}", r.bad);
        assert!(r.bad[0].reason.contains("no provisioned `labels`"));
        assert_eq!(r.exit_code(), 1);
    }

    // rivet: verifies REQ-013
    #[test]
    fn an_exemption_holds_for_a_schedule_only_workflow() {
        let t = wf(
            "on:\n  schedule:\n    - cron: '0 3 * * *'\n  workflow_dispatch:",
            "[self-hosted, linux, light]",
        );
        let r = check_workflow("nightly.yml", &t, &cfg(), true);
        assert_eq!(
            r,
            Report {
                seen: 1,
                bad: vec![]
            }
        );
        let r = check_workflow("nightly.yml", &wf("on: [schedule]", "light"), &cfg(), true);
        assert_eq!(
            r,
            Report {
                seen: 1,
                bad: vec![]
            }
        );
        // Labels are still checked in an exempt workflow.
        let r = check_workflow(
            "nightly.yml",
            &wf("on: [schedule]", "[self-hosted, lihgt]"),
            &cfg(),
            true,
        );
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-013
    #[test]
    fn an_exemption_on_a_pull_request_workflow_is_a_violation() {
        for on in [
            "on: [pull_request]",
            "on:\n  push:\n  pull_request:",
            "on: [merge_group]",
        ] {
            let r = check_workflow("x.yml", &wf(on, "[self-hosted, light]"), &cfg(), true);
            assert_eq!(r.bad.len(), 1, "{on}: {r:?}");
            assert!(r.bad[0].reason.contains("exempt"), "{r:?}");
        }
        // Even with no self-hosted job, the exemption itself is checked.
        let r = check_workflow(
            "x.yml",
            &wf("on: [pull_request]", "ubuntu-24.04"),
            &cfg(),
            true,
        );
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-013
    #[test]
    fn an_exemption_does_not_admit_computed_runners() {
        let r = check_workflow(
            "n.yml",
            &wf("on: [schedule]", "${{ vars.RUNNER }}"),
            &cfg(),
            true,
        );
        assert_eq!(r.bad.len(), 1, "{r:?}");
    }

    // rivet: verifies REQ-013
    #[test]
    fn an_exemption_for_a_missing_workflow_is_a_violation() {
        let dir = tempdir_with("x.yml", &wf("on: [push]", &good("light")));
        let mut c = cfg();
        c.exempt = vec![Exempt {
            workflow: "gone.yml".into(),
            reason: "old".into(),
        }];
        let r = run(dir.path(), &c).unwrap();
        assert_eq!(r.bad.len(), 1, "{:?}", r.bad);
        assert!(r.bad[0].reason.contains("gone.yml"));
    }

    // rivet: verifies REQ-013
    #[test]
    fn config_is_strict() {
        assert!(Config::parse("lables = [\"x\"]").is_err());
        assert!(Config::parse("labels = []").is_err());
        assert!(Config::parse("[[exempt]]\nworkflow = \"a.yml\"\nreason = \" \"").is_err());
        assert!(Config::parse("[[exempt]]\nworkflow = \"../a.yml\"\nreason = \"x\"").is_err());
        let c = Config::parse("labels = [\"self-hosted\"]\n[[exempt]]\nworkflow = \"a.yml\"\nreason = \"schedule only\"").unwrap();
        assert_eq!(c.fallback, DEFAULT_FALLBACK);
        assert_eq!(c.exempt.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn a_tree_with_no_self_hosted_line_compared_nothing() {
        let dir = tempdir_with("x.yml", &wf("on: [push]", "ubuntu-24.04"));
        let r = run(dir.path(), &cfg()).unwrap();
        assert_eq!((r.seen, r.exit_code()), (0, 1));
        assert!(text(&r).contains("compared nothing"));
    }

    // rivet: verifies REQ-011
    #[test]
    fn a_name_naming_self_hosted_is_not_a_site() {
        let t = format!(
            "name: no fork PR reaches a self-hosted runner\non: [pull_request]\njobs:\n  a:\n    name: self-hosted guard\n    runs-on: {}\n    steps:\n      - name: No fork PR reaches a self-hosted runner\n        run: true\n",
            good("light")
        );
        assert_eq!(
            check("t.yml", &t),
            Report {
                seen: 1,
                bad: vec![]
            }
        );
        // A runs-on is still a site even in a job that has a name.
        let t = "on: [pull_request]\njobs:\n  a:\n    name: x\n    runs-on: [self-hosted, light]\n";
        assert_eq!(check("t.yml", t).bad.len(), 1);
    }

    // rivet: verifies REQ-011
    #[test]
    fn release_is_a_trusted_trigger() {
        let r = check(
            "t.yml",
            &wf(
                "on:\n  release:\n    types: [published]\n  push:",
                &good("light"),
            ),
        );
        assert_eq!(
            r,
            Report {
                seen: 1,
                bad: vec![]
            }
        );
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir_with(name: &str, text: &str) -> TempDir {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "plant-guard-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(d.join(".github/workflows")).unwrap();
        fs::write(d.join(".github/workflows").join(name), text).unwrap();
        TempDir(d)
    }
}
