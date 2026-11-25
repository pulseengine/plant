//! scripts/census-collect.sh against a fake `gh` that serves canned API
//! answers from a directory, so no network or login is needed.
//!
//! The fake repository acme/app has branch protection requiring "Test", a
//! ruleset requiring "Lint" and "Test", one workflow, a manifest, and three
//! plant-report artifacts: an older and a newer one from main, and a newest
//! one from a feature branch. The collector must take the newer main one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tmp(name: &str) -> Tmp {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "plant-collect-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&p).unwrap();
    Tmp(p)
}

/// A fake gh: `gh api [-H raw] PATH` prints $FAKE_GH/<PATH with /?&= as _>
/// (with .raw for the raw media type); a .403 file means HTTP 403, and
/// anything else is HTTP 404.
const FAKE_GH: &str = r#"#!/bin/sh
raw=0; path=
for a in "$@"; do
  case $a in
    api|-H) ;;
    'Accept: application/vnd.github.raw') raw=1 ;;
    *) path=$a ;;
  esac
done
f="$FAKE_GH/$(printf %s "$path" | tr '/?&=' '____')"
[ $raw = 1 ] && f="$f.raw"
if [ -f "$f" ]; then cat "$f"; exit 0; fi
if [ -f "$f.403" ]; then echo "gh: Must have admin rights to Repository. (HTTP 403)" >&2; exit 1; fi
echo "gh: Not Found (HTTP 404)" >&2
exit 1
"#;

fn put(dir: &Path, api_path: &str, body: &[u8]) {
    let name: String = api_path
        .chars()
        .map(|c| if "/?&=".contains(c) { '_' } else { c })
        .collect();
    fs::write(dir.join(name), body).unwrap();
}

fn fake_api(t: &Path) -> PathBuf {
    let bin = t.join("bin");
    let api = t.join("api");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&api).unwrap();
    fs::write(bin.join("gh"), FAKE_GH).unwrap();
    Command::new("chmod")
        .arg("+x")
        .arg(bin.join("gh"))
        .status()
        .unwrap();

    let fx = root().join("tests/fixtures/census-collect");
    put(&api, "repos/acme/app", br#"{"default_branch": "main"}"#);
    put(
        &api,
        "repos/acme/app/branches/main/protection/required_status_checks",
        br#"{"contexts": ["Test"]}"#,
    );
    put(
        &api,
        "repos/acme/app/rules/branches/main",
        br#"[{"type": "pull_request"},
             {"type": "required_status_checks", "parameters":
               {"required_status_checks": [{"context": "Lint"}, {"context": "Test"}]}}]"#,
    );
    put(
        &api,
        "repos/acme/app/contents/.github/workflows?ref=main",
        br#"[{"name": "ci.yml", "type": "file"}, {"name": "README.md", "type": "file"}]"#,
    );
    put(
        &api,
        "repos/acme/app/contents/.github/workflows/ci.yml?ref=main.raw",
        b"on: push\njobs:\n  test:\n    name: Test\n    runs-on: ubuntu-24.04\n    steps:\n      - run: cargo test\n",
    );
    put(
        &api,
        "repos/acme/app/contents/ci-controls.toml?ref=main",
        b"{}",
    );
    put(
        &api,
        "repos/acme/app/contents/ci-controls.toml?ref=main.raw",
        b"[[control]]\nid = \"t-red\"\ncheck = \"Test\"\nrun = \"cargo test\"\nplant = \"p\"\nstderr_or_stdout_matches = 'x'\n",
    );
    put(
        &api,
        "repos/acme/app/actions/artifacts?name=plant-report&per_page=100",
        br#"{"artifacts": [
          {"id": 3, "expired": false, "created_at": "2026-11-21T00:00:00Z", "workflow_run": {"head_branch": "feat"}},
          {"id": 2, "expired": false, "created_at": "2026-11-20T00:00:00Z", "workflow_run": {"head_branch": "main"}},
          {"id": 1, "expired": false, "created_at": "2026-11-02T00:00:00Z", "workflow_run": {"head_branch": "main"}}
        ]}"#,
    );
    put(
        &api,
        "repos/acme/app/actions/artifacts/2/zip",
        &fs::read(fx.join("newer.zip")).unwrap(),
    );
    put(
        &api,
        "repos/acme/app/actions/artifacts/1/zip",
        &fs::read(fx.join("older.zip")).unwrap(),
    );
    put(
        &api,
        "repos/acme/app/actions/artifacts/3/zip",
        &fs::read(fx.join("older.zip")).unwrap(),
    );
    api
}

fn collect(t: &Path, api: &Path, out: &Path) -> Output {
    let path = format!(
        "{}:{}",
        t.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    Command::new(root().join("scripts/census-collect.sh"))
        .args([out.to_str().unwrap(), "acme/app"])
        .env("PATH", path)
        .env("FAKE_GH", api)
        .output()
        .unwrap()
}

// rivet: verifies REQ-016
#[test]
fn collect_writes_the_census_inputs_for_a_repository() {
    let t = tmp("ok");
    let api = fake_api(&t.0);
    let out = t.0.join("census");
    let o = collect(&t.0, &api, &out);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{err}");
    let d = out.join("acme/app");
    assert_eq!(
        fs::read_to_string(d.join("repo.toml")).unwrap(),
        "default_branch = \"main\"\nrequired = [\"Lint\",\"Test\"]\n"
    );
    assert!(fs::read_to_string(d.join(".github/workflows/ci.yml"))
        .unwrap()
        .contains("name: Test"));
    assert!(!d.join(".github/workflows/README.md").exists());
    assert!(d.join("ci-controls.toml").exists());
    let report = fs::read_to_string(d.join("report.json")).unwrap();
    assert!(
        report.contains("\"date\": \"2026-11-20\""),
        "collected a report other than the newest from the default branch:\n{report}"
    );

    // plant census reads what the collector wrote.
    let c = Command::new(env!("CARGO_BIN_EXE_plant"))
        .args(["census", out.to_str().unwrap(), "--month", "2026-11"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&c.stdout);
    assert_eq!(
        c.status.code(),
        Some(0),
        "{text}{}",
        String::from_utf8_lossy(&c.stderr)
    );
    assert!(
        text.contains("acme/app: 1 of 2 required contexts covered"),
        "{text}"
    );
}

// An unreadable protection list is unknown, not empty.
// rivet: verifies REQ-016
#[test]
fn collect_stops_when_branch_protection_cannot_be_read() {
    let t = tmp("403");
    let api = fake_api(&t.0);
    let p = "repos_acme_app_branches_main_protection_required_status_checks";
    fs::rename(api.join(p), api.join(format!("{p}.403"))).unwrap();
    let o = collect(&t.0, &api, &t.0.join("census"));
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(
        o.status.code(),
        Some(2),
        "an unreadable branch protection was read as no required checks:\n{err}"
    );
    assert!(err.contains("cannot read"), "{err}");
}
