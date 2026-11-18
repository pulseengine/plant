//! The true inputs planted by examples/sieve's controls, checked against
//! cargo's own build rather than `sieve graph`, so the check is not the
//! classifier judging itself.
//!
//! Each class builds one package: core builds fx-core, proofs builds
//! fx-proofs (and with it fx-core). Building that package and reading the
//! dep-info (.d) of every crate cargo compiled for it gives the files the
//! class really depends on. Then, for the fixture's ci-controls.toml:
//!   - a control expecting `^<class>=true$` must plant only paths in that
//!     class's set;
//!   - the run-all controls and the docs-only baseline must plant paths in
//!     no class's set, so what selects them is sieve's rule, not the graph.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sieve/fixture")
}

/// Repository-relative source files cargo read to build `package`.
fn inputs(package: &str, target: &Path) -> BTreeSet<String> {
    let out = Command::new(env!("CARGO"))
        .args([
            "build",
            "--offline",
            "--quiet",
            "--message-format=json",
            "-p",
            package,
        ])
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("run cargo");
    assert!(out.status.success(), "cargo build -p {package}: {out:?}");
    let root = fixture().canonicalize().unwrap();
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
                    let abs = root.join(dep).canonicalize().unwrap_or_default();
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
    class.insert("core", inputs("fx-core", &target));
    class.insert("proofs", inputs("fx-proofs", &target));
    let _ = fs::remove_dir_all(&target);

    // The oracle sees the indirect inputs the controls rely on.
    for (c, f) in [
        ("core", "shared/table.rs"),
        ("core", "data/limits.txt"),
        ("proofs", "core/src/lib.rs"),
        ("proofs", "proofs/src/lib.rs"),
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
        6,
        "fixture control count changed: update this test"
    );
    let in_any = |p: &str| class.values().any(|s| s.contains(p));
    for c in controls {
        let id = c["id"].as_str().unwrap();
        let regex = c["stderr_or_stdout_matches"].as_str().unwrap();
        let paths = planted(&fixture().join(c["plant"].as_str().unwrap()));
        assert!(!paths.is_empty(), "{id}: plants no path");
        let expected: Vec<&str> = class
            .keys()
            .copied()
            .filter(|k| regex.contains(&format!("^{k}=true$")))
            .collect();
        match expected.as_slice() {
            [k] => {
                for p in &paths {
                    assert!(
                        class[k].contains(p),
                        "{id}: {p} is not an input of {k} by dep-info"
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
