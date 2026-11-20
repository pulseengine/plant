//! `plant guard` through the binary: exit codes, config handling, JSON.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repo(name: &str, workflow: Option<&str>, config: Option<&str>) -> Dir {
    let d = std::env::temp_dir().join(format!("plant-guard-it-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join(".github/workflows")).unwrap();
    if let Some(w) = workflow {
        fs::write(d.join(".github/workflows/ci.yml"), w).unwrap();
    }
    if let Some(c) = config {
        fs::write(d.join("plant-guard.toml"), c).unwrap();
    }
    Dir(d)
}

fn guard(dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_plant"))
        .arg("guard")
        .arg(dir)
        .args(extra)
        .output()
        .unwrap()
}

const GOOD: &str = "on: [pull_request]\njobs:\n  a:\n    runs-on: ${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('[\"self-hosted\",\"light\"]') || 'ubuntu-24.04' }}\n";
const LABELS: &str = "labels = [\"self-hosted\", \"light\"]\n";

// rivet: verifies REQ-011
#[test]
fn a_fail_closed_tree_exits_0() {
    let r = repo("ok", Some(GOOD), Some(LABELS));
    let out = guard(&r.0, &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("OK (1 self-hosted line(s) in 1 workflow(s)")
    );
}

// rivet: verifies REQ-011
#[test]
fn an_unguarded_site_exits_1_and_json_names_it() {
    let r = repo(
        "bad",
        Some("on: [pull_request]\njobs:\n  a:\n    runs-on: [self-hosted, light]\n"),
        Some(LABELS),
    );
    let out = guard(&r.0, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "plant.guard.v1");
    assert_eq!(v["ok"], false);
    assert_eq!(v["violations"][0]["file"], "ci.yml");
    assert_eq!(v["violations"][0]["line"], 4);
    assert_eq!(v["violations"][0]["job"], "a");
}

// rivet: verifies REQ-011
#[test]
fn unreadable_workflows_or_config_exit_2() {
    let r = repo("nowf", None, None);
    fs::remove_dir_all(r.0.join(".github")).unwrap();
    assert_eq!(guard(&r.0, &[]).status.code(), Some(2));
    let r = repo("badcfg", Some(GOOD), Some("lables = []\n"));
    let out = guard(&r.0, &[]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("plant-guard.toml"));
    let r = repo("nocfg", Some(GOOD), None);
    assert_eq!(
        guard(&r.0, &["--config", "/nonexistent/plant-guard.toml"])
            .status
            .code(),
        Some(2)
    );
}

// rivet: verifies REQ-012
#[test]
fn self_hosted_without_declared_labels_exits_1() {
    let r = repo("nolabels", Some(GOOD), None);
    let out = guard(&r.0, &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("no provisioned `labels`"));
}

// rivet: verifies REQ-011
#[test]
fn plants_own_workflows_pass() {
    let out = guard(Path::new(env!("CARGO_MANIFEST_DIR")), &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

/// synth#1768: one runs-on guard stated as an anchor, 56 jobs aliasing it
/// (synth's ci.yml is at the 512 KB workflow limit).
fn aliased(guard_expr: &str, jobs: usize) -> String {
    let mut w = format!("on: [pull_request]\njobs:\n  j0:\n    runs-on: &guard {guard_expr}\n");
    for i in 1..jobs {
        w.push_str(&format!("  j{i}:\n    runs-on: *guard\n"));
    }
    w
}

// rivet: verifies REQ-011
#[test]
fn an_aliased_fail_closed_guard_passes() {
    let expr = "${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('[\"self-hosted\",\"light\"]') || 'ubuntu-24.04' }}";
    let r = repo("alias-ok", Some(&aliased(expr, 56)), Some(LABELS));
    let out = guard(&r.0, &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK (56 self-hosted line(s)"));
}

// rivet: verifies REQ-011
#[test]
fn an_aliased_fail_open_guard_fails_on_every_alias() {
    let expr = "${{ github.event.pull_request.head.repo.fork && 'ubuntu-24.04' || fromJSON('[\"self-hosted\",\"light\"]') }}";
    let r = repo("alias-open", Some(&aliased(expr, 56)), Some(LABELS));
    let out = guard(&r.0, &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("FAIL  56 violation(s)"), "{text}");
    assert!(
        text.contains("(j55)"),
        "the last alias must be judged: {text}"
    );
}
