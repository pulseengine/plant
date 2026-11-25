//! Embedded documentation: `plant docs [TOPIC]`.

pub const TOPICS: &[(&str, &str, &str)] = &[
    ("manifest", "The ci-controls.toml format", MANIFEST),
    ("verdicts", "When a control passes or fails", VERDICTS),
    ("exit-codes", "What plant's exit code means", EXIT_CODES),
    ("mechanics", "Scratch copies, plants, timeouts", MECHANICS),
    (
        "guard",
        "plant guard: no fork PR reaches a self-hosted runner",
        GUARD,
    ),
    (
        "census",
        "plant census: which required checks a plant turned red",
        CENSUS,
    ),
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
  shallow_git = false                            # optional; true gives the copy a depth-1 .git
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
- With shallow_git = true the copy is instead a depth-1 git checkout of HEAD
  (real commit id, no remote, no tags), and the plant is left uncommitted, so
  a check can compare the tree with its base commit (`git diff HEAD`), for
  example a ledger that may only shrink. export-ignore and export-subst do
  not apply to it. It costs a fetch per run.
- A plant file named `<name>.planted` is written as `<name>`: store a planted
  manifest or lockfile (Cargo.lock, package.json, go.sum, ...) as
  `Cargo.lock.planted`. Dependabot and Renovate read those names anywhere in
  the tree, so a deliberately vulnerable planted lock raises an alert and a
  PR that would \"fix\" it, and merging that PR would weaken the control. plant
  warns on stderr about a plant file stored under such a name. A plant that
  holds both `x` and `x.planted` is an error.
- Symlinks in a plant must point inside the scratch copy.
- On SIGINT or SIGTERM plant kills the running check's process group,
  removes its temp dir and exits 2.
- A process that leaves the check's process group (setsid, setpgrp) is not
  killed by plant.
";

const GUARD: &str = r#"plant guard [DIR] [--config FILE] [--format text|json]

Reads DIR/.github/workflows/*.yml and *.yaml and fails unless no fork pull
request can reach a self-hosted runner. Ported from pulseengine/relay's
check-selfhosted-fork-guard.rs (relay#695), itself from ordeal#322.

Accepted runs-on values:
  - a hosted image: ubuntu-*, macos-*, windows-*;
  - the fail-closed form, with the configured fallback image:
      ${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository) && fromJSON('["self-hosted","linux","x64","light"]') || 'ubuntu-24.04' }}
  - its matrix variant, with matrix.runner values that are hosted images or
    label lists containing "self-hosted";
  - ${{ matrix.<key> }} whose every literal value is a hosted image.
Hosted images are exact names (ubuntu-24.04, ubuntu-24.04-arm, macos-15,
windows-2022, the -latest aliases, ...); `ubuntu-gpu` is a label, not an image.
Everything else fails: plain label lists, bare labels, the fail-open
`head.repo.fork && ... ||` form, swapped branches, runners from needs, inputs,
vars or github.*, block-list or map runs-on, a matrix or include computed at
run time, a self-hosted matrix runner outside the matrix form, a job with no
runs-on, a merge key, a reusable workflow from another repository (`uses:`
not starting with ./) unless trusted (below), and a file that does not parse
as YAML. A self-hosted job under a trigger other than push, pull_request, schedule,
workflow_dispatch, merge_group, release or workflow_call fails. A called
workflow runs under its caller's event, so the fail-closed form still sees a
fork pull request; the caller's trigger is checked where the call is made, for local `./`
calls as for trusted remote ones. A run that finds no
self-hosted selection and no trusted workflow call fails: it compared
nothing.

Anchors and aliases are resolved before judging: a guard stated once as
`runs-on: &guard ...` and reused as `runs-on: *guard` is judged at every
alias, so an aliased fail-open guard fails on each job that uses it.

merge_group is trusted as in relay's original: code from a fork runs on
self-hosted once a maintainer queues the pull request.

Config, DIR/plant-guard.toml (or --config):
  fallback = "ubuntu-24.04"            # the hosted image a fork falls back to;
                                       # or a list: ["ubuntu-24.04", "ubuntu-latest"]
  labels = ["self-hosted", "linux", "x64", "light"]   # provisioned labels
  trusted_workflows = ["pulseengine/jig/.github/workflows/*.yml"]
                                       # reusable workflows a job may call
  [[exempt]]                           # may select labels without the form
  workflow = "nightly.yml"
  reason = "schedule only"
Every label in a self-hosted selection must be in `labels`; with self-hosted
lines and no `labels`, the run fails. An exemption holds only when every
trigger of that workflow is push, schedule, workflow_dispatch or release, and
an exemption naming a missing workflow fails. Unknown keys are an error.

A trusted_workflows entry is owner/repo/.github/workflows/<file>.yml (or
.yaml), where <file> may be `*`; no other wildcard and no ref. A job calling
another repository's workflow passes only when the path matches an entry,
the ref is a 40-hex commit SHA (a tag or branch can move), the job has no
runs-on, and every trigger of the calling workflow is in the list above.
The runners the called workflow selects are not visible here: run plant
guard in the repository that owns it.

Exit: 0 all fail-closed, 1 a violation or nothing compared, 2 could not read
the workflows or the config. --format json prints schema plant.guard.v1.
"#;

const CENSUS: &str = r#"plant census DIR --month YYYY-MM [--format text|json]

Counts, per repository, the required checks that a planted input turned red.
It is a pure function of files; it fetches nothing. DIR holds one
OWNER/REPO directory per repository:

  repo.toml              default_branch = "main"
                         required = ["Test", "Lint"]   # the required contexts
  .github/workflows/     the workflows; a job's name (or id) is its context
  ci-controls.toml       the repository's manifest (optional)
  report.json            plant --format json output (schema plant.report.v1),
                         plus a "run" block (optional):
                         {"branch": "main", "date": "2026-11-20",
                          "plant_verified": true}
                         The GitHub Action adds the run block, and with
                         artifact: plant-report uploads the report.

Each required context gets one class:
  covered             a control whose check names the context meets all five:
                      1 bound: its run is a whole command line of a run step
                        in the job that produces the context;
                      2 red: it has stderr_or_stdout_matches, and the report
                        says PASS with a non-zero planted exit;
                      3 green baseline: baseline is on and the report says
                        green;
                      4 fresh: the report is from the default branch and
                        dated in --month;
                      5 verified: the report says the plant binary was a
                        release checked by cosign and sha256.
  declared-not-bound  a control names it, but none meets all five; the
                      reasons list every condition that failed
  aggregate           its job only reads the results of the jobs it needs,
                      so no plant can be bound to it
  uncovered           no control names it

pulseengine/plant is listed as excluded: it runs plant built from the commit
under test. Output schema: plant.census.v1. Exit 0 when the census was
computed, 1 when DIR holds no repository, 2 on unreadable or invalid input.

Not claimed: that a covered check is a good check, or that an uncovered one
is fail-open. A job name built from an expression (a matrix) is not resolved,
so its contexts show as uncovered.
"#;
