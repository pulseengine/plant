//! The true inputs planted by examples/sieve's controls, checked against
//! cargo's own build rather than `sieve graph`, so the check is not the
//! classifier judging itself.
//!
//! Each class builds one package: core builds fx-core, proofs builds
//! fx-proofs (and with it fx-core), gen builds fx-gen in the nested
//! workspace tools/, whose own Cargo.lock pins that build. Building that package and reading the
//! dep-info (.d) of every crate cargo compiled for it gives the files the
//! class really depends on. Then, for the fixture's ci-controls.toml:
//!   - a control expecting `^<class>=true$` must plant only paths in that
//!     class's set;
//!   - the run-all controls and the docs-only baseline must plant paths in
//!     no class's set, so what selects them is sieve's rule, not the graph;
//!   - a planted nested Cargo.lock must be the lockfile of the workspace
//!     its class's package builds in, by `cargo metadata`;
//!   - a planted workflow edit must select exactly the classes of the jobs
//!     it changed, the jobs that need them and the jobs those need, read
//!     from the workflow's own `needs:` graph; a run-all one must change
//!     something outside a job body (outside `jobs:`, or a job's `if:`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use yaml_rust2::{Yaml, YamlLoader};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sieve/fixture")
}

/// Repository-relative source files cargo read to build `package` in the
/// workspace at `ws` (relative to the fixture).
fn inputs(package: &str, ws: &str, target: &Path) -> BTreeSet<String> {
    let out = Command::new(env!("CARGO"))
        .args([
            "build",
            "--offline",
            "--locked",
            "--quiet",
            "--message-format=json",
            "-p",
            package,
        ])
        .current_dir(fixture().join(ws))
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("run cargo");
    assert!(out.status.success(), "cargo build -p {package}: {out:?}");
    let root = fixture().canonicalize().unwrap();
    let ws_root = fixture().join(ws).canonicalize().unwrap();
    let mut files = BTreeSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if msg["reason"] != "compiler-artifact" {
            continue;
        }
        // libfx_core-<hash>.rlib  ->  deps/fx_core-<hash>.d
        for f in msg["filenames"].as_array().into_iter().flatten() {
            let f = PathBuf::from(f.as_str().unwrap());
            let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
            let stem = stem.strip_prefix("lib").unwrap_or(&stem).to_string();
            let d = f.with_file_name(format!("{stem}.d"));
            let Ok(text) = fs::read_to_string(&d) else {
                continue;
            };
            for l in text.lines() {
                let Some((_, deps)) = l.split_once(": ") else {
                    continue;
                };
                // Paths in dep-info are relative to the workspace root.
                for dep in deps.split_whitespace() {
                    let abs = ws_root.join(dep).canonicalize().unwrap_or_default();
                    if let Ok(rel) = abs.strip_prefix(&root) {
                        files.insert(rel.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    assert!(
        !files.is_empty(),
        "no dep-info found for {package}: compared nothing"
    );
    files
}

/// The fixture-relative workspace root cargo uses for the package at `manifest`.
fn workspace_of(manifest: &str) -> String {
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
        ])
        .arg("--manifest-path")
        .arg(fixture().join(manifest))
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "cargo metadata {manifest}: {out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ws = PathBuf::from(doc["workspace_root"].as_str().unwrap());
    let root = fixture().canonicalize().unwrap();
    ws.strip_prefix(&root)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn jobs(text: &str) -> BTreeMap<String, Yaml> {
    let doc = YamlLoader::load_from_str(text).unwrap().remove(0);
    doc["jobs"]
        .as_hash()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str().unwrap().to_string(), v.clone()))
        .collect()
}

fn needs(job: &Yaml) -> Vec<String> {
    match &job["needs"] {
        Yaml::String(s) => vec![s.clone()],
        Yaml::Array(a) => a.iter().map(|n| n.as_str().unwrap().to_string()).collect(),
        _ => Vec::new(),
    }
}

/// For a planted edit of the fixture's workflow: None when it changes
/// something outside a job body (run-all by sieve's rule), else the classes
/// of the changed jobs, the jobs needing them (transitively) and the jobs
/// those need.
fn workflow_classes(
    base: &str,
    planted: &str,
    classes: &BTreeSet<&str>,
) -> Option<BTreeSet<String>> {
    let mut b = YamlLoader::load_from_str(base).unwrap().remove(0);
    let mut p = YamlLoader::load_from_str(planted).unwrap().remove(0);
    let (bj, pj) = (jobs(base), jobs(planted));
    if let (Yaml::Hash(bh), Yaml::Hash(ph)) = (&mut b, &mut p) {
        bh.remove(&Yaml::String("jobs".into()));
        ph.remove(&Yaml::String("jobs".into()));
    }
    if b != p || bj.keys().ne(pj.keys()) {
        return None;
    }
    let changed: BTreeSet<String> = bj.keys().filter(|k| bj[*k] != pj[*k]).cloned().collect();
    if changed.iter().any(|k| bj[k]["if"] != pj[k]["if"]) {
        return None;
    }
    let mut set = changed.clone();
    loop {
        let more: Vec<String> = pj
            .iter()
            .filter(|(k, j)| !set.contains(*k) && needs(j).iter().any(|n| set.contains(n)))
            .map(|(k, _)| k.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        set.extend(more);
    }
    let needed: Vec<String> = set.iter().flat_map(|k| needs(&pj[k])).collect();
    set.extend(needed);
    Some(
        set.into_iter()
            .filter(|k| classes.contains(k.as_str()))
            .collect(),
    )
}

fn planted(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join(".plant/changed.md"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

// rivet: verifies REQ-014
#[test]
fn planted_true_inputs_are_true_inputs_by_cargos_dep_info() {
    let target = std::env::temp_dir().join(format!("plant-sieve-fixture-{}", std::process::id()));
    let mut class: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    class.insert("core", inputs("fx-core", "", &target));
    class.insert("proofs", inputs("fx-proofs", "", &target));
    class.insert("gen", inputs("fx-gen", "tools", &target));
    let _ = fs::remove_dir_all(&target);

    // The oracle sees the indirect inputs the controls rely on.
    for (c, f) in [
        ("core", "shared/table.rs"),
        ("core", "data/limits.txt"),
        ("proofs", "core/src/lib.rs"),
        ("proofs", "proofs/src/lib.rs"),
        ("gen", "tools/gen/src/main.rs"),
    ] {
        assert!(
            class[c].contains(f),
            "dep-info of {c} lacks {f}: {:?}",
            class[c]
        );
    }

    let manifest: toml::Table =
        toml::from_str(&fs::read_to_string(fixture().join("ci-controls.toml")).unwrap()).unwrap();
    let controls = manifest["control"].as_array().unwrap();
    assert_eq!(
        controls.len(),
        10,
        "fixture control count changed: update this test"
    );
    let in_any = |p: &str| class.values().any(|s| s.contains(p));
    // The nested workspace's lockfile, by cargo: gen builds in tools/, and
    // core and proofs build in the root workspace.
    assert_eq!(workspace_of("tools/gen/Cargo.toml"), "tools");
    assert_eq!(workspace_of("core/Cargo.toml"), "");
    let lock_of = |k: &str| match k {
        "gen" => Some("tools/Cargo.lock"),
        _ => None,
    };
    let names: BTreeSet<&str> = class.keys().copied().collect();
    let workflow = ".github/workflows/ci.yml";
    let base_wf = fs::read_to_string(fixture().join(workflow)).unwrap();
    for c in controls {
        let id = c["id"].as_str().unwrap();
        let regex = c["stderr_or_stdout_matches"].as_str().unwrap();
        let dir = fixture().join(c["plant"].as_str().unwrap());
        let expected: Vec<&str> = class
            .keys()
            .copied()
            .filter(|k| regex.contains(&format!("^{k}=true$")))
            .collect();
        if !dir.join(".plant/changed.md").exists() {
            // A planted workflow edit, classified in git mode.
            let planted_wf = fs::read_to_string(dir.join(workflow)).unwrap_or_else(|e| {
                panic!("{id}: plants neither a change list nor {workflow}: {e}")
            });
            assert_eq!(
                c.get("shallow_git").and_then(|v| v.as_bool()),
                Some(true),
                "{id}: git mode needs shallow_git"
            );
            match workflow_classes(&base_wf, &planted_wf, &names) {
                None => assert!(
                    expected.is_empty() && regex.contains("run-all path"),
                    "{id}: the edit is not scoped, so it must run everything"
                ),
                Some(want) => {
                    let got: BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
                    assert!(
                        !want.is_empty(),
                        "{id}: the edit selects no class; it tests nothing"
                    );
                    assert_eq!(
                        got, want,
                        "{id}: the regex's classes differ from the workflow's needs closure"
                    );
                }
            }
            continue;
        }
        let paths = planted(&dir);
        assert!(!paths.is_empty(), "{id}: plants no path");
        match expected.as_slice() {
            [k] => {
                for p in &paths {
                    assert!(
                        class[k].contains(p) || lock_of(k) == Some(p.as_str()),
                        "{id}: {p} is not an input of {k} by dep-info, nor its workspace's lockfile"
                    );
                }
            }
            [] => {
                for p in &paths {
                    assert!(
                        !in_any(p),
                        "{id}: run-all plant {p} is a class input; it would not test the rule"
                    );
                }
            }
            _ => panic!("{id}: expects more than one class"),
        }
    }
    for p in planted(&fixture()) {
        assert!(
            !in_any(&p),
            "baseline path {p} is a class input; the baseline must be docs only"
        );
    }
}
