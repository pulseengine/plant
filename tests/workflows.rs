//! Every CI job keeps fork pull requests off self-hosted runners.

use std::fs;
use std::path::Path;

const GUARD: &str = "runs-on: ${{ (github.event_name != 'pull_request' || \
github.event.pull_request.head.repo.full_name == github.repository) && \
fromJSON('[\"self-hosted\",\"linux\",\"x64\",\"light\"]') || 'ubuntu-latest' }}";

// rivet: verifies REQ-008
#[test]
fn every_job_uses_the_fork_guard() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut jobs = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let text = fs::read_to_string(&p).unwrap();
        assert!(
            !text.contains("pull_request_target"),
            "{} uses pull_request_target",
            p.display()
        );
        for line in text
            .lines()
            .filter(|l| l.trim_start().starts_with("runs-on:"))
        {
            jobs += 1;
            assert_eq!(line.trim(), GUARD, "{}: unguarded runs-on", p.display());
        }
    }
    assert!(jobs > 0, "no jobs found: nothing was compared");
}
