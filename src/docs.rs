//! Embedded documentation: `plant docs [TOPIC]`.

pub const TOPICS: &[(&str, &str, &str)] = &[
    ("manifest", "The ci-controls.toml format", MANIFEST),
    ("verdicts", "When a control passes or fails", VERDICTS),
    ("exit-codes", "What plant's exit code means", EXIT_CODES),
    ("mechanics", "Scratch copies, plants, timeouts", MECHANICS),
];

pub fn list() -> String {
    let mut s = String::from("Topics (plant docs <topic>):\n");
    for (name, about, _) in TOPICS {
        s.push_str(&format!("  {name:<12} {about}\n"));
    }
    s
}

pub fn topic(name: &str) -> Option<&'static str> {
    TOPICS.iter().find(|t| t.0 == name).map(|t| t.2)
}

pub const EXIT_CODES: &str = "\
Exit codes:
  0  every control passed
  1  a control failed, or the manifest has no controls (nothing was compared)
  2  could not run: bad manifest, unknown --only id, git missing, no commit
";

const MANIFEST: &str = r#"ci-controls.toml lives at the repository root: an array of [[control]] tables.

  [[control]]
  id      = "empty-artifact-list"                # unique
  check   = "Verification gate"                  # the CI check it protects (reporting)
  run     = "python3 tools/run_verification.py"  # shell command, run from the scratch root
  plant   = "ci-controls/empty-artifact-list"    # directory overlaid onto the scratch copy
  expect  = "fail"                               # optional; the only accepted value
  stderr_or_stdout_matches = "nothing ran"       # optional regex over the planted output
  timeout  = 300                                 # optional, seconds per run
  baseline = true                                # optional; false skips the green half
  [control.env]                                  # optional extra environment, both runs
  KEY = "value"

Unknown keys, duplicate ids, wrong types, a bad regex or expect other than
"fail" make the manifest invalid (exit 2). The regex uses Rust regex syntax
(no lookaround or backreferences).
"#;

const VERDICTS: &str = "\
A control PASSES only when the baseline (unplanted) run exits 0 and the
planted run exits non-zero for a real reason. It FAILS when:
  - the planted run exits 0 (fail-open);
  - the baseline is red;
  - either run times out (a hang is not a red);
  - the command cannot start (exit 126/127 is not a red);
  - stderr_or_stdout_matches is given and the planted output does not match;
  - the plant directory is missing, or has no files and no deletes;
  - a plant or delete path escapes the root, goes through a symlink, or a
    delete names a path not in the scratch copy.
A planted run killed by a signal is not a red either: an OOM or runner kill
says nothing about the input. A baseline killed by a signal is red.
baseline = false is reported as skipped, never as checked.
";

const MECHANICS: &str = "\
- HEAD is exported once with `git archive` into a temp dir; each run gets its
  own copy. The working tree is never written.
- The baseline runs in one copy; the plant is applied to a fresh copy, because
  a check may write caches or reports.
- The plant is read from the working tree, not HEAD, so a new control can be
  tried before it is committed. Files overlay at the same relative path. A
  file named `delete` at the top of the plant lists paths to remove, one per
  line, `#` comments allowed.
- When tracked files are dirty, plant says so: the copy is HEAD, so those
  edits are not under test.
- Each run is `/bin/sh -c <run>` in its own process group. On timeout, and
  when the run ends, the whole group is killed, so children die too.
- The scratch copy has no .git directory, and is what `git archive` gives:
  export-ignore files are missing, export-subst files are expanded, and
  submodules are absent.
- Symlinks in a plant must point inside the scratch copy.
- On SIGINT or SIGTERM plant kills the running check's process group,
  removes its temp dir and exits 2.
- A process that leaves the check's process group (setsid, setpgrp) is not
  killed by plant.
";
