//! examples/bundle: the evidence-bundle pack (docs/plan/v0.4.0.md, bet C).
//! The proof-packs CI job runs its controls against the bundle's verify.sh
//! with a real wsc. This test keeps the pack honest without wsc:
//! - verify.sh is still the byte copy of wohl v0.7.0's verify.sh;
//! - the unplanted bundle matches its MANIFEST.sha256, so a red planted run
//!   is caused by the plant, not by a stale fixture;
//! - each plant that rewrites MANIFEST.sha256 rewrote it to match the planted
//!   files, so the piece's own check is the one that must reject it.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

const FIXTURE: &str = "examples/bundle/fixture";
/// sha256 of verify.sh in wohl-v0.7.0-evidence-bundle.tar.gz (wohl release
/// v0.7.0, whose SHA256SUMS.txt is cosign-signed by wohl's release workflow).
const WOHL_VERIFY_SHA256: &str = "a2807c63a18637c5e2137426927775e250e5fc1dc7e78fddf3a9adb416922493";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())
        .unwrap();
    assert!(out.status.success(), "cannot hash {}", path.display());
    String::from_utf8(out.stdout).unwrap()[..64].to_string()
}

/// MANIFEST.sha256 as path -> digest, paths without the leading "./".
fn manifest(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(|l| {
            let (digest, path) = l.split_once("  ").expect("sha256sum line");
            (
                path.trim_start_matches("./").to_string(),
                digest.to_string(),
            )
        })
        .collect()
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

// rivet: verifies REQ-017
#[test]
fn verify_sh_is_wohls_v0_7_0_copy() {
    let have = sha256(&root().join(FIXTURE).join("bundle/verify.sh"));
    assert_eq!(
        have, WOHL_VERIFY_SHA256,
        "examples/bundle/fixture/bundle/verify.sh drifted from wohl v0.7.0's verify.sh"
    );
}

// rivet: verifies REQ-017
#[test]
fn bundle_and_rewritten_manifests_match_their_files() {
    let fx = root().join(FIXTURE);
    let bundle = fx.join("bundle");
    let base = manifest(&fs::read_to_string(bundle.join("MANIFEST.sha256")).unwrap());
    let mut files = Vec::new();
    files_under(&bundle, &bundle, &mut files);
    files.retain(|f| f != "MANIFEST.sha256");
    files.sort();
    assert_eq!(
        base.keys().cloned().collect::<Vec<_>>(),
        files,
        "MANIFEST.sha256 does not list exactly the bundle's files"
    );
    for (path, digest) in &base {
        assert_eq!(
            &sha256(&bundle.join(path)),
            digest,
            "{path} is stale in MANIFEST.sha256"
        );
    }

    // A plant that carries its own MANIFEST.sha256 must list the bundle as it
    // is after planting: files it plants or keeps, minus files it deletes.
    let mut checked = 0;
    for e in fs::read_dir(fx.join("ci-controls")).unwrap() {
        let plant = e.unwrap().path();
        let m = plant.join("bundle/MANIFEST.sha256");
        if !m.exists() {
            continue;
        }
        checked += 1;
        let id = plant.file_name().unwrap().to_string_lossy().into_owned();
        let deleted: Vec<String> = fs::read_to_string(plant.join("delete"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim_start_matches("bundle/").to_string())
            .collect();
        let want = manifest(&fs::read_to_string(&m).unwrap());
        for path in base.keys().filter(|p| !deleted.contains(p)) {
            let planted = plant.join("bundle").join(path);
            let file = if planted.exists() {
                planted
            } else {
                bundle.join(path)
            };
            assert_eq!(
                want.get(path),
                Some(&sha256(&file)),
                "plant {id}: MANIFEST.sha256 does not match {path} after planting"
            );
        }
        for path in &deleted {
            assert!(!want.contains_key(path), "plant {id}: lists deleted {path}");
        }
    }
    assert!(
        checked >= 5,
        "only {checked} plants rewrite MANIFEST.sha256"
    );
}
