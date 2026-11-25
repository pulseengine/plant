#!/usr/bin/env bash
# Collects the inputs of `plant census` (see `plant docs census`) from GitHub,
# with the caller's own gh login; no token is stored anywhere.
#
#   scripts/census-collect.sh DIR OWNER/REPO...
#   plant census DIR --month YYYY-MM
#
# For each repository it writes DIR/OWNER/REPO/:
#   repo.toml           the default branch, and the required contexts from
#                       branch protection and from active rulesets;
#   .github/workflows/  the workflow files on the default branch;
#   ci-controls.toml    if the default branch has one;
#   report.json         the newest unexpired `plant-report` artifact of a
#                       default-branch run, if it holds a json report.
#
# Reading classic branch protection needs admin read on the repository. A
# repository whose protection cannot be read stops the run (exit 2): an
# unreadable list is not an empty one. Needs gh, jq and unzip.
set -euo pipefail

die() { echo "census-collect: $*" >&2; exit 2; }
[ $# -ge 2 ] || die "usage: census-collect.sh DIR OWNER/REPO..."
out=$1
shift
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# gh api PATH into FILE; returns 1 for a 404 and exits 2 on any other error.
# Call it as `if fetch ...`, never inside $( ), where its exit would end only
# the subshell.
fetch() {
  if gh api "$1" >"$2" 2>"$work/err"; then
    return 0
  elif true; then
    return 1
  else
    cat "$work/err" >&2
    die "cannot read $1"
  fi
}

for repo in "$@"; do
  case $repo in
    */*) ;;
    *) die "$repo is not OWNER/REPO" ;;
  esac
  dir="$out/$repo"
  rm -rf "$dir"
  mkdir -p "$dir/.github/workflows"

  fetch "repos/$repo" "$work/repo.json" || die "$repo not found"
  branch=$(jq -r .default_branch "$work/repo.json")

  : >"$work/contexts"
  if fetch "repos/$repo/branches/$branch/protection/required_status_checks" "$work/bp.json"; then
    jq -r '.contexts[]' "$work/bp.json" >>"$work/contexts"
  fi
  if fetch "repos/$repo/rules/branches/$branch" "$work/rules.json"; then
    jq -r '.[] | select(.type == "required_status_checks")
      | .parameters.required_status_checks[].context' "$work/rules.json" >>"$work/contexts"
  fi
  {
    printf 'default_branch = %s\n' "$(jq -n --arg b "$branch" '$b')"
    printf 'required = %s\n' "$(sort -u "$work/contexts" | jq -R . | jq -sc .)"
  } >"$dir/repo.toml"

  if fetch "repos/$repo/contents/.github/workflows?ref=$branch" "$work/wf.json"; then
    jq -r '.[] | select(.type == "file") | select(.name | test("\\.ya?ml$")) | .name' "$work/wf.json" |
      while IFS= read -r f; do
        gh api -H 'Accept: application/vnd.github.raw' \
          "repos/$repo/contents/.github/workflows/$f?ref=$branch" >"$dir/.github/workflows/$f"
      done
  fi

  if fetch "repos/$repo/contents/ci-controls.toml?ref=$branch" "$work/m.json"; then
    gh api -H 'Accept: application/vnd.github.raw' \
      "repos/$repo/contents/ci-controls.toml?ref=$branch" >"$dir/ci-controls.toml"
  fi

  report=none
  if fetch "repos/$repo/actions/artifacts?name=plant-report&per_page=100" "$work/a.json"; then
    id=$(jq -r --arg b "$branch" '[.artifacts[] | select(.expired | not)
      | select(.workflow_run.head_branch == $b)] | sort_by(.created_at) | last | .id // empty' "$work/a.json")
    if [ -n "$id" ]; then
      gh api "repos/$repo/actions/artifacts/$id/zip" >"$work/a.zip"
      unzip -p "$work/a.zip" plant-report >"$work/report" 2>/dev/null || true
      if [ "$(head -c 1 "$work/report" 2>/dev/null)" = "{" ]; then
        mv "$work/report" "$dir/report.json"
        report="artifact $id"
      else
        report="artifact $id is not a json report"
      fi
    fi
  fi
  echo "census-collect: $repo ($branch): $(sort -u "$work/contexts" | wc -l | tr -d ' ') required context(s), report: $report"
done
