//! Text and JSON rendering of a report.

use serde_json::json;

use crate::control::Report;

pub fn text(r: &Report) -> String {
    let mut lines = Vec::new();
    for c in &r.results {
        lines.push(format!(
            "{}  {}  [{}]: {}",
            c.status, c.id, c.check, c.reason
        ));
        if c.baseline == "skipped" {
            lines.push(
                "      note: baseline = false, so the real input staying green is NOT checked"
                    .into(),
            );
        }
        if c.status == "FAIL" {
            for l in c.output_tail.lines() {
                lines.push(format!("      | {l}"));
            }
        }
    }
    lines.push(format!(
        "{} control(s): {} passed, {} failed",
        r.results.len(),
        r.passed(),
        r.failed()
    ));
    if r.results.is_empty() {
        lines.push("FAIL  the manifest has no controls: nothing was compared".into());
    }
    lines.join("\n")
}

pub fn json(r: &Report) -> String {
    let doc = json!({
        "controls": r.results,
        "summary": {
            "total": r.results.len(),
            "passed": r.passed(),
            "failed": r.failed(),
            "empty_manifest": r.results.is_empty(),
            "tracked_changes_not_under_test": r.tracked_changes,
        },
    });
    serde_json::to_string_pretty(&doc).expect("serializable")
}
