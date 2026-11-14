//! The verdict rules of docs/DESIGN.md, as pure functions.
//!
//! Kept in one small file on purpose: plant's own ci-controls.toml plants
//! broken copies of this file and expects the self-tests to go red.

use regex::Regex;

/// How one run of a control's command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    NotStarted(String),
}

/// What the plant directory holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlantState {
    Missing,
    Empty,
    Ready,
}

/// A plant must exist and change something.
pub fn check_plant(state: &PlantState, plant: &str) -> Result<(), String> {
    match state {
        PlantState::Missing => Ok(()),
        PlantState::Empty => Err(format!(
            "plant dir {plant} holds no files and no deletes, so it plants nothing"
        )),
        PlantState::Ready => Ok(()),
    }
}

/// The unplanted run must be green, or the planted red proves nothing.
pub fn judge_baseline(outcome: &Outcome, timeout: f64) -> Result<(), String> {
    match outcome {
        Outcome::Exited(0) => Ok(()),
        Outcome::Exited(c @ (126 | 127)) => {
            Err(format!("baseline command could not start (exit {c})"))
        }
        Outcome::Exited(c) => Err(format!(
            "baseline is red (exit {c}) before anything is planted"
        )),
        Outcome::Signaled(s) => Err(format!(
            "baseline is red (killed by signal {s}) before anything is planted"
        )),
        Outcome::TimedOut => Err(format!("baseline timed out after {timeout}s")),
        Outcome::NotStarted(e) => Err(format!("baseline command could not start: {e}")),
    }
}

/// The planted run must be a real rejection: a non-zero exit, not a hang,
/// not a kill by a signal (an OOM or runner kill is not the check judging the
/// input), not a command that could not start, and matching the regex when
/// one is given.
pub fn judge_planted(
    outcome: &Outcome,
    timeout: f64,
    regex: Option<&Regex>,
    output: &str,
) -> Result<String, String> {
    let red = match outcome {
        Outcome::TimedOut => {
            return Err(format!(
                "planted run timed out after {timeout}s (a hang is not a red)"
            ))
        }
        Outcome::NotStarted(e) => {
            return Err(format!("command could not start ({e}), which is not a red"))
        }
        Outcome::Exited(c @ (126 | 127)) => {
            return Err(format!(
                "command could not start (exit {c}), which is not a red"
            ))
        }
        Outcome::Exited(0) => {
            return Err(
                "FAIL-OPEN: planted run exited 0, the check did not notice the plant".into(),
            )
        }
        Outcome::Signaled(s) => {
            return Err(format!(
                "planted run was killed by signal {s}, which is not a red"
            ))
        }
        Outcome::Exited(c) => format!("exited {c}"),
    };
    match regex {
        Some(re) if !re.is_match(output) => Err(format!(
            "planted run {red} but the output does not match /{re}/, \
             so it went red for some other reason"
        )),
        Some(_) => Ok(format!("planted run {red}, output matched")),
        None => Ok(format!("planted run {red}")),
    }
}

/// Exit 0 only when at least one control ran and every control passed.
pub fn overall(passed: usize, failed: usize) -> u8 {
    if passed + failed == 0 || failed > 0 {
        1
    } else {
        0
    }
}
