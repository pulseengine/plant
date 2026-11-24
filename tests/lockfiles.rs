//! The lockfile gate's controls (ci-controls.lockfiles.toml) plant bad locks
//! in a nested workspace. CI's lockfiles job runs them against
//! scripts/lockfiles.sh; this test keeps their target real: the nested
//! workspace exists, has a committed lock, and every plant touches it.

use std::fs;
use std::path::Path;
use std::process::Command;

const NESTED: &str = "examples/sieve/fixture/tools";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

// rivet: verifies REQ-031
#[test]
fn lock_controls_target_a_committed_nested_workspace() {
    let ws = fs::read_to_string(root().join(NESTED).join("Cargo.toml")).unwrap();
    assert!(
        ws.contains("[workspace]"),
        "{NESTED} is no longer a workspace"
    );
    let tracked = Command::new("git")
        .args([
            "ls-files",
            "--error-unmatch",
            &format!("{NESTED}/Cargo.lock"),
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        tracked.status.success(),
        "{NESTED}/Cargo.lock is not committed"
    );

    let text = fs::read_to_string(root().join("ci-controls.lockfiles.toml")).unwrap();
    let doc: toml::Table = text.parse().unwrap();
    let controls = doc["control"].as_array().unwrap();
    assert_eq!(controls.len(), 3);
    for c in controls {
        let id = c["id"].as_str().unwrap();
        assert_eq!(c["run"].as_str(), Some("scripts/lockfiles.sh"), "{id}");
        let regex = c["stderr_or_stdout_matches"].as_str().unwrap();
        assert!(
            regex.contains(r"tools/Cargo\.lock"),
            "{id}: regex must name the nested lock"
        );
        let plant = root().join(c["plant"].as_str().unwrap());
        let touches = plant.join(NESTED).exists()
            || fs::read_to_string(plant.join("delete"))
                .map(|d| {
                    d.lines()
                        .any(|l| l.trim() == format!("{NESTED}/Cargo.lock"))
                })
                .unwrap_or(false);
        assert!(touches, "{id}: plant does not touch {NESTED}");
    }
}

// The lockfile pack's gate is a copy of plant's own, so that adopters can take
// the fixture as it is. It may not drift: the pack would then prove a gate
// plant no longer runs.
// rivet: verifies REQ-034
#[test]
fn locks_pack_gate_is_plants_lockfile_gate() {
    let ours = fs::read(root().join("scripts/lockfiles.sh")).unwrap();
    let pack = fs::read(root().join("examples/locks/fixture/check.sh")).unwrap();
    assert!(
        ours == pack,
        "examples/locks/fixture/check.sh differs from scripts/lockfiles.sh; copy it over"
    );
}
