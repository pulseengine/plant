//! The GitHub Action wrapper: action/run.sh and action.yml.

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
        "plant-action-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&p).unwrap();
    Tmp(p)
}

/// A stand-in for plant: records one argument per line, exits with $FAKE_RC.
fn fake_plant(dir: &Path) -> PathBuf {
    let p = dir.join("fake-plant");
    fs::write(
        &p,
        "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\"; done > \"$FAKE_ARGS\"\n\
         echo fake report\nexit \"${FAKE_RC:-0}\"\n",
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    p
}

fn run_sh(dir: &Path, env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(root().join("action/run.sh"));
    c.current_dir(dir)
        .env_remove("GITHUB_EVENT_NAME")
        .env_remove("GITHUB_STEP_SUMMARY")
        .env("PLANT_BIN", fake_plant(dir))
        .env("FAKE_ARGS", dir.join("args"))
        .env("PLANT_REPORT", dir.join("report"));
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

// rivet: verifies REQ-009
#[test]
fn refuses_pull_request_target() {
    let t = tmp("prt");
    let out = run_sh(&t.0, &[("GITHUB_EVENT_NAME", "pull_request_target")]);
    // First, so a missing guard fails with this message and nothing else.
    assert!(
        !t.0.join("args").exists(),
        "plant ran under pull_request_target"
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("pull_request_target"));
    // The twin: an ordinary pull_request runs plant.
    let out = run_sh(&t.0, &[("GITHUB_EVENT_NAME", "pull_request")]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(t.0.join("args").exists());
}

// rivet: verifies REQ-009
#[test]
fn inputs_reach_plant_as_arguments() {
    let t = tmp("args");
    let summary = t.0.join("summary.md");
    let out = run_sh(
        &t.0,
        &[
            ("PLANT_MANIFEST", "m.toml"),
            ("PLANT_ONLY", "a  b *"),
            ("PLANT_FORMAT", "json"),
            ("GITHUB_STEP_SUMMARY", summary.to_str().unwrap()),
            ("FAKE_RC", "1"),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "plant's exit code must pass through"
    );
    let args = fs::read_to_string(t.0.join("args")).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        ["m.toml", "--format", "json", "--only", "a", "--only", "b", "--only", "*"]
    );
    assert_eq!(
        fs::read_to_string(t.0.join("report")).unwrap(),
        "fake report\n"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("fake report"));
    let s = fs::read_to_string(&summary).unwrap();
    assert!(
        s.contains("plant (exit 1)") && s.contains("fake report"),
        "{s}"
    );
}

// Expressions in action.yml may only feed env: values, the outputs map, or
// the name and path inputs of the upload step, never a run: script, where an
// input could inject shell code.
// rivet: verifies REQ-009
#[test]
fn action_yml_keeps_expressions_out_of_scripts() {
    let text = fs::read_to_string(root().join("action.yml")).unwrap();
    let env_line =
        regex::Regex::new(r"^\s+[A-Z_]+: \$\{\{ [a-z_.]+ \}\}[A-Za-z0-9_./-]*$").unwrap();
    let value_line = regex::Regex::new(r"^\s+value: \$\{\{ [a-z_.]+ \}\}$").unwrap();
    let upload_line =
        regex::Regex::new(r"^\s+(name|path): \$\{\{ [a-z_.]+ \}\}[A-Za-z0-9_./-]*$").unwrap();
    let mut seen = 0;
    for line in text.lines().filter(|l| l.contains("${{")) {
        seen += 1;
        assert!(
            env_line.is_match(line) || value_line.is_match(line) || upload_line.is_match(line),
            "expression outside env/value: {line:?}"
        );
    }
    assert!(seen > 0, "no expressions found: nothing was compared");
}

// The upload step uploads the file the run step writes, and also after a red
// run: a failed control's report is the one a reader needs (plan bet B).
// rivet: verifies REQ-016
#[test]
fn action_uploads_the_report_it_writes_even_when_plant_fails() {
    use yaml_rust2::YamlLoader;
    let text = fs::read_to_string(root().join("action.yml")).unwrap();
    let doc = &YamlLoader::load_from_str(&text).unwrap()[0];
    let steps = doc["runs"]["steps"].as_vec().unwrap();
    let run = steps
        .iter()
        .find(|s| s["id"].as_str() == Some("run"))
        .expect("no run step");
    let written = run["env"]["PLANT_REPORT"].as_str().unwrap();
    let uploads: Vec<_> = steps
        .iter()
        .filter(|s| {
            s["uses"]
                .as_str()
                .is_some_and(|u| u.starts_with("actions/upload-artifact@"))
        })
        .collect();
    assert_eq!(uploads.len(), 1, "expected one upload step");
    let up = uploads[0];
    assert_eq!(
        up["with"]["path"].as_str(),
        Some(written),
        "the upload step uploads another file than the run step writes"
    );
    assert_eq!(up["with"]["name"].as_str(), Some("${{ inputs.artifact }}"));
    let cond = up["if"].as_str().unwrap_or("");
    assert!(
        cond.starts_with("always() &&") && cond.contains("inputs.artifact != ''"),
        "the report upload is skipped when plant fails: if = {cond:?}"
    );
}

/// A stand-in for plant --format json: a pretty report as plant prints it.
fn fake_json_plant(dir: &Path) -> PathBuf {
    let p = dir.join("fake-json-plant");
    fs::write(
        &p,
        "#!/bin/sh\nprintf '{\\n  \"schema\": \"plant.report.v1\",\\n  \"controls\": []\\n}\\n'\n",
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    p
}

// A json report from the action carries the run block plant census reads;
// plant_verified is true only when a release version was installed.
// rivet: verifies REQ-016
#[test]
fn json_report_gets_the_census_run_block() {
    for (version, verified) in [("0.5.0", true), ("", false)] {
        let t = tmp("runblock");
        let bin = fake_json_plant(&t.0);
        let out = run_sh(
            &t.0,
            &[
                ("PLANT_BIN", bin.to_str().unwrap()),
                ("PLANT_FORMAT", "json"),
                ("PLANT_VERSION", version),
                ("GITHUB_REF_NAME", "ma\"in"),
            ],
        );
        assert!(out.status.success(), "{out:?}");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(t.0.join("report")).unwrap()).unwrap();
        assert_eq!(v["schema"], "plant.report.v1");
        assert_eq!(v["run"]["branch"], "ma\"in");
        assert_eq!(v["run"]["plant_verified"], verified, "version {version:?}");
        let date = v["run"]["date"].as_str().unwrap();
        assert!(date.len() == 10 && date.as_bytes()[4] == b'-', "{date}");
    }
}

// The verified install pins the signer to plant's own release workflow for
// exactly the requested tag; no environment variable can change it.
// rivet: verifies REQ-016
#[test]
fn install_pins_the_signer_identity_to_the_tag() {
    let text = fs::read_to_string(root().join("action/install.sh")).unwrap();
    assert!(text.contains(
        "identity=\"https://github.com/pulseengine/plant/.github/workflows/release.yml@refs/tags/$tag\""
    ));
    assert!(text.contains("--certificate-identity \"$identity\""));
    assert!(!text.contains("certificate-identity-regexp"));
}
