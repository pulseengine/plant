//! `plant north-star`: the North Star indicator input plant attaches to each
//! release (seam pulseengine.eu#393, schema data/north-star/input.schema.json
//! schema_version 1, pinned at pulseengine.eu 21bf3cd in
//! tests/fixtures/north-star-input.schema.json).
//!
//! The facts come from a committed file (north-star/O3.toml). This command
//! checks the honesty rules the seam sets and writes the JSON:
//!   - a fact that did not run cannot pass;
//!   - a fact that ran names its evidence and says whether its planted
//!     control turned it red;
//!   - a fact passes only when its planted control turned it red.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const OBJECTIVES: &[&str] = &["O1", "O2", "O3", "O4", "O5"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsFile {
    objective: String,
    #[serde(default)]
    not_measured: Vec<String>,
    #[serde(default)]
    fact: Vec<Fact>,
}

#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: String,
    pub ran: bool,
    #[serde(default)]
    pub pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_failed_as_expected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<toml::Table>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// A release version as the schema accepts it: `v?MAJOR.MINOR.PATCH[-pre|+build]`.
fn valid_version(v: &str) -> bool {
    let v = v.strip_prefix('v').unwrap_or(v);
    let (core, rest) = match v.find(['-', '+']) {
        Some(i) => (&v[..i], &v[i + 1..]),
        None => (v, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let num = |p: &&str| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    parts.len() == 3
        && parts.iter().all(num)
        && (v.len() == core.len()
            || (!rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')))
}

/// UTC now as RFC 3339, without a date crate.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    rfc3339(secs)
}

fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

/// Builds the input document, or says which honesty rule a fact breaks.
pub fn build(text: &str, version: &str, generated_at: &str) -> Result<serde_json::Value, String> {
    let f: FactsFile = toml::from_str(text).map_err(|e| e.to_string())?;
    if !OBJECTIVES.contains(&f.objective.as_str()) {
        return Err(format!(
            "objective {:?} is not one of {OBJECTIVES:?}",
            f.objective
        ));
    }
    if !valid_version(version) {
        return Err(format!("version {version:?} is not vMAJOR.MINOR.PATCH"));
    }
    if f.not_measured.is_empty() {
        return Err("not_measured is empty: name what this input is blind to".into());
    }
    let mut seen = BTreeSet::new();
    for x in &f.fact {
        if x.id.trim().is_empty() || !seen.insert(x.id.clone()) {
            return Err(format!("fact id {:?} is empty or repeated", x.id));
        }
        if !x.ran && (x.pass || x.control_failed_as_expected.is_some() || x.evidence.is_some()) {
            return Err(format!(
                "{}: a fact that did not run has no result, control or evidence",
                x.id
            ));
        }
        if x.ran {
            if x.evidence
                .as_deref()
                .map_or(true, |e| !e.starts_with("https://"))
            {
                return Err(format!(
                    "{}: a fact that ran names its evidence as an https URL",
                    x.id
                ));
            }
            if x.control_failed_as_expected.is_none() {
                return Err(format!(
                    "{}: a fact that ran says whether its planted control turned it red",
                    x.id
                ));
            }
            if x.pass && x.control_failed_as_expected != Some(true) {
                return Err(format!(
                    "{}: a fact passes only when its planted control turned it red",
                    x.id
                ));
            }
        }
    }
    let facts: Vec<serde_json::Value> = f
        .fact
        .iter()
        .map(|x| serde_json::to_value(x).expect("fact serialises"))
        .collect();
    Ok(serde_json::json!({
        "schema_version": 1,
        "objective": f.objective,
        "repo": "pulseengine/plant",
        "version": version,
        "generated_at": generated_at,
        "generator": {"tool": "plant", "version": env!("CARGO_PKG_VERSION")},
        "facts": facts,
        "not_measured": f.not_measured,
    }))
}

pub fn run(input: &Path, version: &str, generated_at: Option<&str>) -> Result<String, String> {
    let text = fs::read_to_string(input)
        .map_err(|e| format!("could not read {}: {e}", input.display()))?;
    let at = generated_at.map(str::to_string).unwrap_or_else(now_rfc3339);
    let doc = build(&text, version, &at).map_err(|e| format!("{}: {e}", input.display()))?;
    Ok(serde_json::to_string_pretty(&doc).expect("json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "objective = \"O3\"\nnot_measured = [\"x\"]\n";

    #[test]
    fn rfc3339_known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_795_132_800), "2026-11-20T00:00:00Z");
    }

    #[test]
    fn versions() {
        for v in ["v0.4.0", "0.4.0", "v1.2.3-rc.1", "v1.2.3+b.5"] {
            assert!(valid_version(v), "{v}");
        }
        for v in ["v0.4", "v0.4.0-", "v0.4.0 ", "x0.4.0", "v0.4.0-rc_1"] {
            assert!(!valid_version(v), "{v}");
        }
    }

    // rivet: verifies REQ-015
    #[test]
    fn honesty_rules_are_enforced() {
        let ok = |facts: &str| build(&format!("{BASE}{facts}"), "v0.4.0", "2026-11-20T00:00:00Z");
        assert!(ok("[[fact]]\nid = \"a\"\nran = false\n").is_ok());
        assert!(ok("[[fact]]\nid = \"a\"\nran = true\npass = true\ncontrol_failed_as_expected = true\nevidence = \"https://x\"\n").is_ok());
        for (bad, why) in [
            ("[[fact]]\nid = \"a\"\nran = false\npass = true\n", "did not run"),
            ("[[fact]]\nid = \"a\"\nran = true\npass = true\ncontrol_failed_as_expected = true\n", "evidence"),
            ("[[fact]]\nid = \"a\"\nran = true\npass = false\nevidence = \"https://x\"\n", "planted control"),
            ("[[fact]]\nid = \"a\"\nran = true\npass = true\ncontrol_failed_as_expected = false\nevidence = \"https://x\"\n", "passes only"),
            ("[[fact]]\nid = \"a\"\nran = false\n[[fact]]\nid = \"a\"\nran = false\n", "repeated"),
        ] {
            let e = ok(bad).err().unwrap_or_default();
            assert!(e.contains(why), "{bad}: {e}");
        }
        assert!(build(
            "objective = \"O9\"\nnot_measured = [\"x\"]\n",
            "v0.4.0",
            "t"
        )
        .is_err());
        assert!(build("objective = \"O3\"\n", "v0.4.0", "t").is_err());
        assert!(build(BASE, "0.4", "t").is_err());
        assert!(build(&format!("{BASE}extra = 1\n"), "v0.4.0", "t").is_err());
    }
}
