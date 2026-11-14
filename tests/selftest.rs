//! The self-test cases of docs/DESIGN.md, driven through the binary against
//! throwaway git repositories, in both directions: a potent check gives PASS
//! and its broken twin gives FAIL.
//!
//! Every run through `Fixture::run` also asserts that the working tree is
//! byte-identical afterwards and that plant left nothing in its TMPDIR.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

const CHECK_SH: &str = r#"#!/bin/sh
echo "written by the check" > leak.txt
mode=${1:-data}
case "$mode" in
  always-ok) exit 0 ;;
  always-red) echo "always red"; exit 1 ;;
  sleep) sleep 60; exit 0 ;;
  needs-file)
    if [ ! -f required.txt ]; then echo "required.txt is gone"; exit 1; fi
    exit 0 ;;
esac
if grep -q bad data.txt; then echo "data.txt is bad: rule R7 violated"; exit 1; fi
echo ok
"#;

struct Fixture {
    base: PathBuf,
    repo: PathBuf,
    tmp: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(st.status.success(), "git {args:?}: {st:?}");
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let ft = e.file_type().unwrap();
            if p.file_name().unwrap() == ".git" {
                continue;
            }
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            if ft.is_symlink() {
                let t = fs::read_link(&p).unwrap();
                out.insert(rel, t.to_string_lossy().as_bytes().to_vec());
            } else if ft.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("bad json ({e}): {}\n{}", self.stdout, self.stderr))
    }
    /// The one control in a JSON report, by id.
    fn control(&self, id: &str) -> Value {
        let doc = self.json();
        doc["controls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no control {id}: {}", self.stdout))
    }
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "plant-selftest-{name}-{}-{nanos}",
            std::process::id()
        ));
        let repo = base.join("repo");
        let tmp = base.join("tmp");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&tmp).unwrap();
        git(&repo, &["init", "-q"]);
        write(&repo.join("check.sh"), CHECK_SH);
        write(&repo.join("data.txt"), "all fine\n");
        write(&repo.join("required.txt"), "here\n");
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "fixture"]);
        // Plants live in the working tree and are deliberately not committed.
        write(&repo.join("plants/bad/data.txt"), "bad\n");
        write(&repo.join("plants/ok/data.txt"), "still fine\n");
        write(
            &repo.join("plants/delete-required/delete"),
            "# drop it\nrequired.txt\n",
        );
        write(&repo.join("plants/escape/delete"), "../outside.txt\n");
        write(
            &repo.join("plants/comment-only/delete"),
            "# only a comment\n",
        );
        fs::create_dir_all(repo.join("plants/empty")).unwrap();
        Fixture { base, repo, tmp }
    }

    fn manifest(&self, name: &str, text: &str) {
        write(&self.repo.join(name), text);
    }

    /// Run plant from the repo root; assert the tree and TMPDIR invariants.
    fn run(&self, args: &[&str]) -> Run {
        let before = snapshot(&self.repo);
        let out = Command::new(env!("CARGO_BIN_EXE_plant"))
            .args(args)
            .current_dir(&self.repo)
            .env("TMPDIR", &self.tmp)
            .output()
            .unwrap();
        let after = snapshot(&self.repo);
        let run = Run {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        };
        assert!(
            before == after,
            "the working tree changed during {args:?}: {}",
            run.stdout
        );
        assert!(
            !self.repo.join("leak.txt").exists(),
            "a check's write leaked"
        );
        let left: Vec<_> = fs::read_dir(&self.tmp).unwrap().collect();
        assert!(left.is_empty(), "plant left scratch behind: {left:?}");
        run
    }

    fn run_json(&self, manifest: &str) -> Run {
        self.run(&[manifest, "--format", "json"])
    }
}

fn ctl(id: &str, run: &str, plant: &str, extra: &str) -> String {
    format!(
        "[[control]]\nid = {}\ncheck = \"Fake check\"\nrun = {}\nplant = {}\n{extra}\n",
        Value::from(id),
        Value::from(run),
        Value::from(plant)
    )
}

const DATA: &str = "sh check.sh data";

fn assert_verdict(c: &Value, status: &str, needle: &str) {
    assert_eq!(c["status"], status, "{c}");
    let reason = c["reason"].as_str().unwrap();
    assert!(
        reason.contains(needle),
        "{needle:?} not in reason {reason:?}"
    );
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_dead(pid: i32) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

// ── verdicts, both directions ───────────────────────────────────────────────

// rivet: verifies REQ-001
#[test]
fn good_check_fails_on_plant() {
    let f = Fixture::new("good");
    f.manifest("m.toml", &ctl("good", DATA, "plants/bad", ""));
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("good"), "PASS", "exited 1");
    assert_eq!(r.control("good")["baseline"], "green");
    assert_eq!(r.code, 0);
}

// rivet: verifies REQ-001
#[test]
fn regex_must_match() {
    let f = Fixture::new("regex");
    f.manifest(
        "m.toml",
        &(ctl(
            "match",
            DATA,
            "plants/bad",
            "stderr_or_stdout_matches = \"rule R7\"",
        ) + &ctl(
            "mismatch",
            DATA,
            "plants/bad",
            "stderr_or_stdout_matches = \"rule R99\"",
        )),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("match"), "PASS", "output matched");
    assert_verdict(&r.control("mismatch"), "FAIL", "does not match");
    assert_eq!(r.code, 1);
}

// rivet: verifies REQ-001
#[test]
fn fail_open_is_fail() {
    let f = Fixture::new("failopen");
    f.manifest(
        "m.toml",
        &(ctl("open", "sh check.sh always-ok", "plants/bad", "")
            + &ctl("ignored", DATA, "plants/ok", "")
            + &ctl("good", DATA, "plants/bad", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("open"), "FAIL", "FAIL-OPEN");
    assert_eq!(r.control("open")["planted_exit"], 0);
    assert_verdict(&r.control("ignored"), "FAIL", "FAIL-OPEN");
    assert_verdict(&r.control("good"), "PASS", "exited 1");
    assert_eq!(r.code, 1, "any failed control makes the run exit 1");
}

// rivet: verifies REQ-001
#[test]
fn baseline_red_is_fail() {
    let f = Fixture::new("baseline");
    f.manifest(
        "m.toml",
        &(ctl("red", "sh check.sh always-red", "plants/bad", "")
            + &ctl("green", DATA, "plants/bad", "")),
    );
    let r = f.run_json("m.toml");
    let red = r.control("red");
    assert_verdict(&red, "FAIL", "baseline is red");
    assert_eq!(red["baseline"], "red (exit 1)");
    assert!(red["output_tail"].as_str().unwrap().contains("always red"));
    assert_verdict(&r.control("green"), "PASS", "exited 1");
}

// rivet: verifies REQ-001
#[test]
fn baseline_false_is_reported_skipped() {
    let f = Fixture::new("skipped");
    f.manifest(
        "m.toml",
        &ctl(
            "skip",
            "sh check.sh always-red",
            "plants/bad",
            "baseline = false",
        ),
    );
    let r = f.run_json("m.toml");
    let c = r.control("skip");
    assert_verdict(&c, "PASS", "exited 1");
    assert_eq!(c["baseline"], "skipped");
    let t = f.run(&["m.toml"]);
    assert!(t.stdout.contains("baseline = false"), "{}", t.stdout);
}

// rivet: verifies REQ-001
#[test]
fn missing_plant_is_fail() {
    let f = Fixture::new("missing");
    f.manifest(
        "m.toml",
        &(ctl("missing", DATA, "plants/does-not-exist", "")
            + &ctl("present", DATA, "plants/bad", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("missing"), "FAIL", "missing");
    assert_verdict(&r.control("present"), "PASS", "exited 1");
}

// rivet: verifies REQ-001
#[test]
fn empty_plants_are_fail() {
    let f = Fixture::new("empty");
    f.manifest(
        "m.toml",
        &(ctl("empty", DATA, "plants/empty", "")
            + &ctl("comments", DATA, "plants/comment-only", "")
            + &ctl(
                "delete-only",
                "sh check.sh needs-file",
                "plants/delete-required",
                "",
            )),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("empty"), "FAIL", "plants nothing");
    assert_verdict(&r.control("comments"), "FAIL", "plants nothing");
    assert_verdict(&r.control("delete-only"), "PASS", "exited 1");
}

// rivet: verifies REQ-001
// rivet: verifies REQ-004
#[test]
fn escaping_paths_are_fail() {
    let f = Fixture::new("escape");
    f.manifest(
        "m.toml",
        &(ctl("delete-escapes", DATA, "plants/escape", "")
            + &ctl("plant-escapes", DATA, "../outside", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("delete-escapes"), "FAIL", "escapes");
    assert_verdict(&r.control("plant-escapes"), "FAIL", "escapes");
}

// rivet: verifies REQ-004
#[test]
fn plant_cannot_write_through_a_symlink() {
    let f = Fixture::new("symlink");
    let outside = f.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.repo.join("out")).unwrap();
    git(&f.repo, &["add", "out"]);
    git(&f.repo, &["commit", "-q", "-m", "symlink"]);
    write(&f.repo.join("plants/through-link/out/evil.txt"), "x\n");
    f.manifest(
        "m.toml",
        &(ctl("through-link", DATA, "plants/through-link", "")
            + &ctl("good", DATA, "plants/bad", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("through-link"), "FAIL", "symlink");
    assert_verdict(&r.control("good"), "PASS", "exited 1");
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

// rivet: verifies REQ-001
#[test]
fn command_not_found_is_fail() {
    let f = Fixture::new("notfound");
    f.manifest(
        "m.toml",
        &ctl(
            "nf",
            "no-such-command-plant-xyz",
            "plants/bad",
            "baseline = false",
        ),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("nf"), "FAIL", "could not start");
}

// rivet: verifies REQ-001
// rivet: verifies REQ-005
#[test]
fn timeout_is_fail_and_kills_the_group() {
    let f = Fixture::new("timeout");
    let pidfile = f.base.join("child.pid");
    let run = "sleep 60 & echo $! > \"$PIDFILE\"; wait";
    let env = format!(
        "timeout = 1\nbaseline = false\n[control.env]\nPIDFILE = {}",
        Value::from(pidfile.to_str().unwrap())
    );
    f.manifest(
        "m.toml",
        &(ctl("hang", run, "plants/bad", &env) + &ctl("quick", DATA, "plants/bad", "timeout = 30")),
    );
    let started = Instant::now();
    let r = f.run_json("m.toml");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "plant waited for the hang"
    );
    assert_verdict(&r.control("hang"), "FAIL", "timed out");
    assert_verdict(&r.control("quick"), "PASS", "exited 1");
    let pid: i32 = fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_dead(pid),
        "the shell's child {pid} survived the timeout"
    );
}

// rivet: verifies REQ-005
#[test]
fn background_children_die_when_the_run_ends() {
    let f = Fixture::new("background");
    let pidfile = f.base.join("bg.pid");
    let env = format!(
        "baseline = false\n[control.env]\nPIDFILE = {}",
        Value::from(pidfile.to_str().unwrap())
    );
    f.manifest(
        "m.toml",
        &ctl(
            "bg",
            "sleep 60 & echo $! > \"$PIDFILE\"; exit 1",
            "plants/bad",
            &env,
        ),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("bg"), "PASS", "exited 1");
    let pid: i32 = fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(wait_dead(pid), "background child {pid} outlived its run");
}

// rivet: verifies REQ-001
#[test]
fn control_env_reaches_the_command() {
    let f = Fixture::new("env");
    f.manifest(
        "m.toml",
        &ctl(
            "env",
            "[ \"$PLANT_T\" = 1 ] && exit 3; exit 0",
            "plants/bad",
            "baseline = false\n[control.env]\nPLANT_T = \"1\"",
        ),
    );
    let r = f.run_json("m.toml");
    let c = r.control("env");
    assert_verdict(&c, "PASS", "exited 3");
    assert_eq!(c["planted_exit"], 3);
}

// A planted file must look newer than anything the baseline wrote, or an
// mtime-cached build (cargo) skips it and the check never sees the plant.
// Found by running plant on itself.
// rivet: verifies REQ-004
#[test]
fn planted_files_are_newer_than_the_baseline() {
    let f = Fixture::new("mtime");
    let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(f.repo.join("plants/bad/data.txt"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    let stamp = f.base.join("baseline.stamp");
    let run = "if [ -e \"$STAMP\" ]; then \
               if [ \"$STAMP\" -nt data.txt ]; then echo stale plant; exit 0; fi; \
               echo plant is fresh; exit 1; fi; touch \"$STAMP\"";
    let extra = format!(
        "[control.env]\nSTAMP = {}",
        Value::from(stamp.to_str().unwrap())
    );
    f.manifest("m.toml", &ctl("fresh", run, "plants/bad", &extra));
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("fresh"), "PASS", "exited 1");
}

// An OOM or runner kill is not the check judging the input.
// rivet: verifies REQ-001
#[test]
fn signal_killed_planted_run_is_fail() {
    let f = Fixture::new("signal");
    let run = "grep -q bad data.txt && kill -9 $$; exit 0";
    f.manifest(
        "m.toml",
        &(ctl("killed", run, "plants/bad", "") + &ctl("good", DATA, "plants/bad", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("killed"), "FAIL", "signal 9");
    assert_verdict(&r.control("good"), "PASS", "exited 1");
}

// rivet: verifies REQ-001
#[test]
fn baseline_timeout_is_fail() {
    let f = Fixture::new("basetimeout");
    f.manifest(
        "m.toml",
        &(ctl("slow", "sh check.sh sleep", "plants/bad", "timeout = 1")
            + &ctl("good", DATA, "plants/bad", "timeout = 30")),
    );
    let r = f.run_json("m.toml");
    let slow = r.control("slow");
    assert_verdict(&slow, "FAIL", "baseline timed out");
    assert_eq!(slow["baseline"], "timeout");
    assert_verdict(&r.control("good"), "PASS", "exited 1");
}

// rivet: verifies REQ-004
#[test]
fn planted_symlinks_must_stay_inside() {
    let f = Fixture::new("plantlink");
    let outside = f.base.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let mk = |dir: &str, target: &Path| {
        let d = f.repo.join("plants").join(dir);
        fs::create_dir_all(&d).unwrap();
        std::os::unix::fs::symlink(target, d.join("out")).unwrap();
    };
    mk("abs", &outside);
    mk("rel", Path::new("../../../outside"));
    mk("ok", Path::new("data.txt"));
    let run = "echo x > out 2>/dev/null; echo y > out/written 2>/dev/null; \
               grep -q bad data.txt && exit 1; [ -L out ] && exit 1; exit 0";
    f.manifest(
        "m.toml",
        &(ctl("abs", run, "plants/abs", "")
            + &ctl("rel", run, "plants/rel", "")
            + &ctl("ok", run, "plants/ok", "")),
    );
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("abs"), "FAIL", "absolute");
    assert_verdict(&r.control("rel"), "FAIL", "outside the scratch copy");
    assert_verdict(&r.control("ok"), "PASS", "exited 1");
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

// rivet: verifies REQ-004
#[test]
fn read_only_dirs_from_a_check_are_cleaned() {
    let f = Fixture::new("readonly");
    let run = "mkdir -p ro && touch ro/f && chmod 555 ro; sh check.sh data";
    f.manifest("m.toml", &ctl("ro", run, "plants/bad", ""));
    // Fixture::run asserts TMPDIR is empty afterwards.
    let r = f.run_json("m.toml");
    assert_verdict(&r.control("ro"), "PASS", "exited 1");
}

// rivet: verifies REQ-005
#[test]
fn sigterm_kills_the_check_and_cleans_up() {
    let f = Fixture::new("sigterm");
    let pidfile = f.base.join("term.pid");
    let env = format!(
        "baseline = false\n[control.env]\nPIDFILE = {}",
        Value::from(pidfile.to_str().unwrap())
    );
    f.manifest(
        "m.toml",
        &ctl(
            "hang",
            "sleep 60 & echo $! > \"$PIDFILE\"; wait",
            "plants/bad",
            &env,
        ),
    );
    let child = Command::new(env!("CARGO_BIN_EXE_plant"))
        .arg("m.toml")
        .current_dir(&f.repo)
        .env("TMPDIR", &f.tmp)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while !pidfile.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid: i32 = fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signalling the plant process this test spawned.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(wait_dead(pid), "the check's child {pid} outlived plant");
    assert!(
        fs::read_dir(&f.tmp).unwrap().next().is_none(),
        "temp dir left"
    );
}

// ── manifest-level behaviour and exit codes ─────────────────────────────────

// rivet: verifies REQ-002
#[test]
fn all_green_exits_0_and_only_narrows() {
    let f = Fixture::new("green");
    f.manifest(
        "good.toml",
        &(ctl("g1", DATA, "plants/bad", "")
            + &ctl("g2", "sh check.sh needs-file", "plants/delete-required", "")),
    );
    let r = f.run(&["good.toml"]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(
        r.stdout.contains("2 control(s): 2 passed, 0 failed"),
        "{}",
        r.stdout
    );
    let r = f.run(&["good.toml", "--only", "g2"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("1 control(s)"), "{}", r.stdout);
    let r = f.run(&["good.toml", "--only", "nope"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("nope"), "{}", r.stderr);
}

// rivet: verifies REQ-002
#[test]
fn empty_manifest_exits_1() {
    let f = Fixture::new("emptymanifest");
    f.manifest("empty.toml", "# no controls\n");
    let r = f.run(&["empty.toml"]);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert!(r.stdout.contains("nothing was compared"), "{}", r.stdout);
    let j = f.run_json("empty.toml");
    assert_eq!(j.code, 1);
    assert_eq!(j.json()["summary"]["empty_manifest"], true);
}

// rivet: verifies REQ-002
// rivet: verifies REQ-003
#[test]
fn bad_manifests_exit_2() {
    let f = Fixture::new("badmanifest");
    let one = ctl("x", DATA, "plants/bad", "");
    let cases = [
        ("garbled.toml", "[[control]\nid = ".to_string()),
        ("dup.toml", format!("{one}{one}")),
        (
            "expect.toml",
            ctl("x", DATA, "plants/bad", "expect = \"pass\""),
        ),
        (
            "unknown.toml",
            ctl("x", DATA, "plants/bad", "colour = \"red\""),
        ),
    ];
    for (name, text) in &cases {
        f.manifest(name, text);
        let r = f.run(&[name]);
        assert_eq!(r.code, 2, "{name}: {}{}", r.stdout, r.stderr);
    }
    assert_eq!(f.run(&["absent.toml"]).code, 2);
}

// rivet: verifies REQ-002
#[test]
fn no_git_history_exits_2() {
    let f = Fixture::new("nogit");
    let bare = f.base.join("nogit");
    write(&bare.join("p/f"), "x");
    write(&bare.join("ci-controls.toml"), &ctl("x", "true", "p", ""));
    let out = Command::new(env!("CARGO_BIN_EXE_plant"))
        .current_dir(&bare)
        .env("TMPDIR", &f.tmp)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(fs::read_dir(&f.tmp).unwrap().next().is_none());
}

// rivet: verifies REQ-004
#[test]
fn dirty_tracked_files_are_reported() {
    let f = Fixture::new("dirty");
    f.manifest("m.toml", &ctl("good", DATA, "plants/bad", ""));
    let clean = f.run_json("m.toml");
    assert_eq!(
        clean.json()["summary"]["tracked_changes_not_under_test"],
        false
    );
    write(&f.repo.join("check.sh"), "exit 0\n");
    let r = f.run_json("m.toml");
    // The copy is HEAD, so the committed check still runs and still passes.
    assert_verdict(&r.control("good"), "PASS", "exited 1");
    assert_eq!(r.json()["summary"]["tracked_changes_not_under_test"], true);
    assert!(r.stderr.contains("not under test"), "{}", r.stderr);
}

// rivet: verifies REQ-006
#[test]
fn text_report_shows_reason_and_output_tail() {
    let f = Fixture::new("text");
    f.manifest(
        "m.toml",
        &(ctl("red", "sh check.sh always-red", "plants/bad", "")
            + &ctl("good", DATA, "plants/bad", "")),
    );
    let r = f.run(&["m.toml"]);
    assert_eq!(r.code, 1);
    assert!(
        r.stdout
            .contains("FAIL  red  [Fake check]: baseline is red"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("      | always red"), "{}", r.stdout);
    assert!(
        r.stdout.contains("PASS  good  [Fake check]"),
        "{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("2 control(s): 1 passed, 1 failed"),
        "{}",
        r.stdout
    );
}

// ── CLI conventions ─────────────────────────────────────────────────────────

fn plant(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_plant"))
        .args(args)
        .output()
        .unwrap()
}

// rivet: verifies REQ-002
// rivet: verifies REQ-006
#[test]
fn cli_conventions() {
    let v = plant(&["--version"]);
    assert_eq!(v.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&v.stdout).trim(),
        format!("plant {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(plant(&["-V"]).status.code(), Some(0));
    let h = plant(&["--help"]);
    assert_eq!(h.status.code(), Some(0));
    let help = String::from_utf8_lossy(&h.stdout);
    assert!(help.contains("--format"), "{help}");
    assert!(help.lines().all(|l| l.len() <= 100), "{help}");
    let bad = plant(&["--no-such-flag"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(!bad.stderr.is_empty());
    assert_eq!(plant(&["docs"]).status.code(), Some(0));
    for t in ["manifest", "verdicts", "exit-codes", "mechanics"] {
        assert_eq!(plant(&["docs", t]).status.code(), Some(0), "docs {t}");
    }
    assert_eq!(plant(&["docs", "nope"]).status.code(), Some(2));
}
