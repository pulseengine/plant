//! plant guard as a WASI component (REQ-018). The CI job "guard (wasi)" runs
//! scripts/guard-wasi.sh and ci-controls.wasi.toml with wasmtime; this test
//! keeps those controls honest without wasmtime.

use std::fs;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

const EXIT_CALL: &str = "    #[cfg(target_os = \"wasi\")]\n    wasi_exit(code);\n";

// rivet: verifies REQ-018
#[test]
fn exit_collapse_plant_is_main_without_the_exit_with_code_call() {
    let main = read("src/main.rs");
    assert_eq!(
        main.matches(EXIT_CALL).count(),
        1,
        "src/main.rs must call wasi_exit once"
    );
    assert_eq!(
        read("wasi-controls/exit-collapse/src/main.rs"),
        main.replace(EXIT_CALL, ""),
        "wasi-controls/exit-collapse/src/main.rs is stale: copy src/main.rs without the \
         wasi_exit call"
    );
}

// rivet: verifies REQ-018
#[test]
fn wasi_manifest_names_each_plant_and_the_test_that_goes_red() {
    let m: toml::Table = read("ci-controls.wasi.toml").parse().unwrap();
    let controls = m["control"].as_array().unwrap();
    let mut plants: Vec<&str> = controls
        .iter()
        .map(|c| {
            assert!(c.get("stderr_or_stdout_matches").is_some(), "{c:?}");
            c["plant"].as_str().unwrap()
        })
        .collect();
    plants.sort();
    let mut dirs: Vec<String> = fs::read_dir(root().join("wasi-controls"))
        .unwrap()
        .map(|e| format!("wasi-controls/{}", e.unwrap().file_name().to_string_lossy()))
        .collect();
    dirs.sort();
    assert_eq!(plants, dirs, "ci-controls.wasi.toml vs wasi-controls/");
    assert!(
        read(".github/workflows/ci.yml").contains("target/debug/plant ci-controls.wasi.toml"),
        "the guard (wasi) job must run ci-controls.wasi.toml"
    );
}

// rivet: verifies REQ-018
#[test]
// CI does not run it yet: kiln 0.9.0 hangs on x86_64 Linux (kiln#748, fixed by
// kiln#737 in v0.9.1). The guard (kiln) job returns with that release.
fn kiln_manifest_reuses_a_wasi_plant() {
    let m: toml::Table = read("ci-controls.kiln.toml").parse().unwrap();
    let controls = m["control"].as_array().unwrap();
    assert!(
        !controls.is_empty(),
        "ci-controls.kiln.toml has no controls"
    );
    for c in controls {
        assert!(c.get("stderr_or_stdout_matches").is_some(), "{c:?}");
        let plant = c["plant"].as_str().unwrap();
        assert!(
            plant.starts_with("wasi-controls/") && root().join(plant).is_dir(),
            "{plant} is not a wasi-controls plant"
        );
    }
}
