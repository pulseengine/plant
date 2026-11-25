//! `plant census` on the committed three-repository fixture in
//! tests/fixtures/census must match expected.json byte for byte
//! (docs/plan/v0.4.0.md, bet B acceptance).
//!
//! The fixture covers each class and each failed condition once:
//! - acme/alpha: Test covered; Lint uncovered; all-green an aggregate, though
//!   a control claims it;
//! - acme/beta: a stale report; Build's control runs a command its job does
//!   not; Docs' control has no regex and baseline = false;
//! - acme/gamma: a report from a feature branch by an unverified plant, whose
//!   planted run exited 0;
//! - pulseengine/plant: excluded.
//!
//! After an intended change, regenerate with
//! `PLANT_REGEN=1 cargo test --test census`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn census(dir: &Path, month: &str) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_plant"))
        .args([
            "census",
            dir.to_str().unwrap(),
            "--month",
            month,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

// rivet: verifies REQ-016
#[test]
fn census_matches_the_committed_expected_file() {
    let dir = root().join("tests/fixtures/census");
    let (code, out, err) = census(&dir, "2026-11");
    assert_eq!(code, 0, "{err}");
    let expected = dir.join("expected.json");
    if std::env::var_os("PLANT_REGEN").is_some() {
        fs::write(&expected, &out).unwrap();
    }
    assert!(
        out == fs::read_to_string(&expected).unwrap(),
        "census differs from tests/fixtures/census/expected.json:\n{out}"
    );
}

// rivet: verifies REQ-016
#[test]
fn an_empty_census_compares_nothing_and_exits_1() {
    let t = std::env::temp_dir().join(format!("plant-census-empty-{}", std::process::id()));
    fs::create_dir_all(&t).unwrap();
    let (code, _, err) = census(&t, "2026-11");
    fs::remove_dir_all(&t).unwrap();
    assert_eq!(code, 1);
    assert!(err.contains("nothing compared"), "{err}");
}

// rivet: verifies REQ-016
#[test]
fn a_bad_month_or_report_schema_is_exit_2() {
    let dir = root().join("tests/fixtures/census");
    let (code, _, err) = census(&dir, "2026-13");
    assert_eq!(code, 2, "{err}");

    let t = std::env::temp_dir().join(format!("plant-census-schema-{}", std::process::id()));
    let r = t.join("o/r");
    fs::create_dir_all(&r).unwrap();
    fs::write(
        r.join("repo.toml"),
        "default_branch = \"main\"\nrequired = [\"Test\"]\n",
    )
    .unwrap();
    fs::write(r.join("report.json"), "{\"schema\": \"plant.report.v0\"}").unwrap();
    let (code, _, err) = census(&t, "2026-11");
    fs::remove_dir_all(&t).unwrap();
    assert_eq!(code, 2);
    assert!(err.contains("plant.report.v1"), "{err}");
}
