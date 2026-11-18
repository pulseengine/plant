# Example: sigil's fork-guard check as plant controls

sigil's `verification/selfhosted-guard/check.py` proves its own rules with an
embedded `--self-test`: synthetic workflows in a temp dir. This manifest turns
those cases into plant controls that run on sigil's real tree instead:

- the baseline runs the guard on sigil's own `.github/workflows` and must be
  green;
- each plant adds one workflow, `.github/workflows/planted.yml`, that the guard
  must reject, and the regex requires the rejection for that job, so a YAML
  error or a crash does not count;
- `compared-nothing` deletes `.github/workflows`, and the guard must refuse to
  pass on nothing.

To use it, copy `ci-controls.toml` and `ci-controls/` to the root of sigil and
run `plant`. This is an example in plant's repository; sigil has not adopted it.

Last run against sigil `main` at f51d09b on 2026-11-17: 12 of 12
controls PASS. While writing it, four plants first went red on a YAML quoting
error in the plant itself; the regex reported them as FAIL ("went red for some
other reason") instead of letting them pass.
