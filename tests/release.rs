//! scripts/release-check.sh: the gates release.yml runs before publishing.

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
        "plant-release-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&p).unwrap();
    Tmp(p)
}

fn check(args: &[&str], env: &[(&str, &Path)]) -> Output {
    let mut c = Command::new(root().join("scripts/release-check.sh"));
    c.args(args).current_dir(root());
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

// rivet: verifies REQ-010
#[test]
fn version_guard_rejects_a_mismatch() {
    let t = tmp("guard");
    let toml = t.0.join("Cargo.toml");
    fs::write(&toml, "[package]\nname = \"plant\"\nversion = \"1.2.3\"\n").unwrap();
    let env = [("CARGO_TOML", toml.as_path())];
    let bad = check(&["version-guard", "v9.9.9"], &env);
    assert_eq!(bad.status.code(), Some(1), "tag v9.9.9 accepted: {bad:?}");
    let good = check(&["version-guard", "v1.2.3"], &env);
    assert_eq!(good.status.code(), Some(0), "{good:?}");
    // The real manifest, as the release job reads it.
    let real = check(&["version-guard", &tag()], &[]);
    assert_eq!(real.status.code(), Some(0), "{real:?}");
}

// rivet: verifies REQ-010
#[test]
fn smoke_accepts_the_built_binary() {
    let out = check(&["smoke", env!("CARGO_BIN_EXE_plant"), &tag()], &[]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

// rivet: verifies REQ-010
#[test]
fn smoke_rejects_a_binary_of_another_version() {
    let out = check(&["smoke", env!("CARGO_BIN_EXE_plant"), "v9.9.9"], &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a binary of another version passed the smoke test: {out:?}"
    );
}

// rivet: verifies REQ-010
#[test]
fn smoke_rejects_a_binary_without_verdicts() {
    // Right version, right --help and unknown-flag exits, but every run is
    // "green": the fixture's fail-open control must catch it.
    let t = tmp("fake");
    let fake = t.0.join("plant");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'plant {}';;\n  --no-such-flag) exit 2;;\nesac\nexit 0\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&fake).status().unwrap();
    let out = check(&["smoke", fake.to_str().unwrap(), &tag()], &[]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("fail-open control did not exit 1"));
}
