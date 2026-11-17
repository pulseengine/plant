# plant

Prove your CI checks can fail.

A green CI check means "nothing I looked at was wrong". It does not say whether
the check looked. `plant` runs each check against an input it must reject, a
planted bad input, and fails when the check stays green. It also runs the check
on the real input first and fails if that baseline is already red, and it fails
when there is nothing to compare.

**Status: work in progress, pre-1.0.** The interface below is the design target.

## The idea in one picture

```text
known-bad input (planted) --> the check --> red?   yes: the check can fail
                                            green? no:  the check is fail-open
```

## Manifest

Each repository lists its controls in `ci-controls.toml`:

```toml
[[control]]
id     = "empty-artifact-list"
check  = "Verification gate"                 # the CI check this protects
run    = "python3 tools/run_verification.py" # command, run from a scratch copy
plant  = "ci-controls/empty-artifact-list"   # files overlaid onto the copy
expect = "fail"
stderr_or_stdout_matches = "nothing ran"     # optional
```

`plant` never touches your working tree: each control runs in a scratch copy of
`HEAD`.

## Running it

```sh
plant                      # ./ci-controls.toml
plant path/to/manifest.toml --only empty-artifact-list --format json
plant docs verdicts        # embedded docs: manifest, verdicts, exit-codes, mechanics
```

Exit codes: 0 every control passed; 1 a control failed or nothing was
compared; 2 plant could not run.

## GitHub Action

```yaml
jobs:
  ci-controls:
    # plant runs the manifest's commands: keep fork pull requests off
    # self-hosted runners.
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pulseengine/plant@<commit-sha>
        with:
          manifest: ci-controls.toml   # default
          only: ""                      # space-separated ids, default all
          format: text                  # or json
```

The action builds plant from its own source (the runner needs Rust, git, tar
and bash), refuses to run under `pull_request_target`, and fails the step with
plant's exit code. Pin it by commit SHA; there is no release yet.

## Licence

Apache-2.0.
