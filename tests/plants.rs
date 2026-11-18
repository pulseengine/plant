//! plant's own controls (ci-controls.toml) plant broken copies of source files.
//! Each plant is its source file with exactly one replacement applied. This
//! test keeps the three in step, both ways:
//!   - every plant file equals its source with the one replacement applied;
//!   - the plant directories, the table below and ci-controls.toml name the
//!     same set of ids.
//!
//! After changing a planted source file, regenerate the plants with
//! `PLANT_REGEN=1 cargo test --test plants`.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// (control id, source file, text replaced, replacement)
const PLANTS: &[(&str, &str, &str, &str)] = &[
    (
        "fail-open",
        "src/verdict.rs",
        "        Outcome::Exited(0) => {
            return Err(
                \"FAIL-OPEN: planted run exited 0, the check did not notice the plant\".into(),
            )
        }
",
        "",
    ),
    (
        "red-baseline",
        "src/verdict.rs",
        "        Outcome::Exited(c) => Err(format!(
            \"baseline is red (exit {c}) before anything is planted\"
        )),
",
        "        Outcome::Exited(_) => Ok(()),
",
    ),
    (
        "missing-plant",
        "src/verdict.rs",
        "        PlantState::Missing => Err(format!(
            \"plant dir {plant} is missing, so nothing was planted\"
        )),
",
        "        PlantState::Missing => Ok(()),
",
    ),
    (
        "timeout",
        "src/verdict.rs",
        "        Outcome::TimedOut => {
            return Err(format!(
                \"planted run timed out after {timeout}s (a hang is not a red)\"
            ))
        }
",
        "        Outcome::TimedOut => \"timed out\".to_string(),
",
    ),
    (
        "empty-manifest",
        "src/verdict.rs",
        "    if passed + failed == 0 || failed > 0 {",
        "    if failed > 0 {",
    ),
    (
        "action-allows-prt",
        "action/run.sh",
        "if [ \"${GITHUB_EVENT_NAME:-}\" = \"pull_request_target\" ]; then",
        "if false; then",
    ),
    (
        "release-tag-mismatch",
        "scripts/release-check.sh",
        "  [ \"$tag\" = \"v$v\" ] || die \"tag $tag does not name Cargo.toml's version $v\"",
        "  true",
    ),
    (
        "release-wrong-version",
        "scripts/release-check.sh",
        "  [ \"$got\" = \"plant ${tag#v}\" ] || die \"binary reports '$got', tag is $tag\"",
        "  true",
    ),
    (
        "kill-shell-only",
        "src/runner.rs",
        "libc::kill(-pgid, libc::SIGKILL);",
        "libc::kill(pgid, libc::SIGKILL);",
    ),
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

// rivet: verifies REQ-007
#[test]
fn plants_match_their_source() {
    let regen = std::env::var_os("PLANT_REGEN").is_some();
    for (id, file, old, new) in PLANTS {
        let src = fs::read_to_string(root().join(file)).unwrap();
        assert_eq!(
            src.matches(old).count(),
            1,
            "plant {id}: the replaced text must occur exactly once in {file}"
        );
        let want = src.replacen(old, new, 1);
        let path = root().join("ci-controls").join(id).join(file);
        let mode = fs::metadata(root().join(file)).unwrap().permissions();
        if regen {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &want).unwrap();
            fs::set_permissions(&path, mode.clone()).unwrap();
        }
        let have = fs::read_to_string(&path).unwrap_or_default();
        assert!(
            have == want,
            "plant {id} is stale against {file}; run PLANT_REGEN=1 cargo test --test plants"
        );
        // A plant that loses the executable bit fails for the wrong reason.
        let have_mode = fs::metadata(&path).unwrap().permissions();
        assert_eq!(
            have_mode.mode() & 0o111,
            mode.mode() & 0o111,
            "plant {id}: executable bits differ from {file}"
        );
    }
}

fn files_under(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files_under(&p, base, out);
        } else {
            out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
        }
    }
}

// rivet: verifies REQ-007
#[test]
fn plant_sets_agree() {
    let table: BTreeSet<&str> = PLANTS.iter().map(|p| p.0).collect();
    assert!(!table.is_empty(), "no plants: nothing would be compared");

    let dirs: BTreeSet<String> = fs::read_dir(root().join("ci-controls"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let dirs: BTreeSet<&str> = dirs.iter().map(String::as_str).collect();
    assert_eq!(table, dirs, "plant table vs ci-controls/ directories");

    for (id, file, _, _) in PLANTS {
        let base = root().join("ci-controls").join(id);
        let mut files = Vec::new();
        files_under(&base, &base, &mut files);
        assert_eq!(
            files,
            vec![file.to_string()],
            "plant {id} holds other files"
        );
    }

    let manifest: toml::Table = fs::read_to_string(root().join("ci-controls.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let controls = manifest["control"].as_array().unwrap();
    let ids: BTreeSet<&str> = controls.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(table, ids, "plant table vs ci-controls.toml ids");
    for c in controls {
        let id = c["id"].as_str().unwrap();
        assert_eq!(
            c["plant"].as_str(),
            Some(format!("ci-controls/{id}").as_str())
        );
        assert!(
            c.get("stderr_or_stdout_matches").is_some(),
            "control {id} must say which test goes red"
        );
    }
}
