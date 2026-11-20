//! `plant north-star` output against pulseengine.eu's input schema, pinned in
//! tests/fixtures/north-star-input.schema.json (pulseengine.eu 21bf3cd). The
//! schema's own required keys, properties, const, enum and patterns are read
//! from that file at test time, so moving the pin moves the check.

use std::path::Path;
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn schema() -> serde_json::Value {
    let text = std::fs::read_to_string(root().join("tests/fixtures/north-star-input.schema.json"))
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    k.sort();
    k
}

fn generate(input: &Path, version: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_plant"))
        .args(["north-star"])
        .arg(input)
        .args([
            "--version",
            version,
            "--generated-at",
            "2026-11-20T00:00:00Z",
        ])
        .output()
        .unwrap()
}

// rivet: verifies REQ-015
#[test]
fn the_committed_o3_input_conforms_to_the_pinned_schema() {
    let out = generate(&root().join("north-star/O3.toml"), "v0.4.0");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = schema();
    let props = &s["properties"];

    // Exactly the required keys, and nothing the schema does not allow.
    assert_eq!(keys(&doc), strings(&s["required"]));
    assert!(keys(&doc).iter().all(|k| props.get(k).is_some()));
    assert_eq!(doc["schema_version"], props["schema_version"]["const"]);
    assert!(props["objective"]["enum"]
        .as_array()
        .unwrap()
        .contains(&doc["objective"]));
    assert_eq!(doc["objective"], "O3");
    for (field, pat) in [
        ("repo", &props["repo"]["pattern"]),
        ("version", &props["version"]["pattern"]),
    ] {
        let re = regex::Regex::new(pat.as_str().unwrap()).unwrap();
        assert!(
            re.is_match(doc[field].as_str().unwrap()),
            "{field}: {}",
            doc[field]
        );
    }
    assert_eq!(
        keys(&doc["generator"]),
        strings(&props["generator"]["required"])
    );

    let item = &props["facts"]["items"];
    let allowed = keys(&item["properties"]);
    let required = strings(&item["required"]);
    let facts = doc["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 21, "7 repositories x 3 kinds");
    for f in facts {
        let k = keys(f);
        assert!(required.iter().all(|r| k.contains(r)), "{f}");
        assert!(k.iter().all(|x| allowed.contains(x)), "{f}");
        // Until a pack control has run against the repository, nothing passes.
        assert_eq!(f["ran"], false, "{f}");
        assert_eq!(f["pass"], false, "{f}");
    }
    assert!(!doc["not_measured"].as_array().unwrap().is_empty());
}

// rivet: verifies REQ-015
#[test]
fn a_fact_that_claims_a_pass_without_its_control_is_refused() {
    let dir = std::env::temp_dir().join(format!("plant-ns-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("O3.toml");
    std::fs::write(
        &f,
        "objective = \"O3\"\nnot_measured = [\"x\"]\n[[fact]]\nid = \"a\"\nran = true\npass = true\ncontrol_failed_as_expected = false\nevidence = \"https://example.invalid/run\"\n",
    )
    .unwrap();
    let out = generate(&f, "v0.4.0");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("passes only when its planted control turned it red"));
}
