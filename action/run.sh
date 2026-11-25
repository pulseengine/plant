#!/usr/bin/env bash
# Runs plant for the GitHub Action in action.yml. Every input arrives as an
# environment variable; nothing from the event payload is interpolated here.
set -euo pipefail
set -f # no globbing of --only ids

if [ "${GITHUB_EVENT_NAME:-}" = "pull_request_target" ]; then
  echo "plant: refusing to run under pull_request_target: it would run the" \
    "pull request's code with the base repository's token and secrets" >&2
  exit 2
fi

args=("${PLANT_MANIFEST:-ci-controls.toml}" --format "${PLANT_FORMAT:-text}")
for id in ${PLANT_ONLY:-}; do
  args+=(--only "$id")
done

report="${PLANT_REPORT:-plant-report.txt}"
rc=0
"${PLANT_BIN:-plant}" "${args[@]}" >"$report" || rc=$?

# A json report gets the run block plant census reads (plant docs census):
# the branch, the UTC date, and whether this plant is a verified release
# (installed by action/install.sh because a version was given).
if [ "${PLANT_FORMAT:-text}" = json ] && [ "$(head -c 1 "$report")" = "{" ]; then
  verified=false
  [ -n "${PLANT_VERSION:-}" ] && verified=true
  branch=${GITHUB_REF_NAME:-}
  branch=${branch//\\/\\\\}
  branch=${branch//\"/\\\"}
  run_line="  \"run\": {\"branch\": \"$branch\", \"date\": \"$(date -u +%Y-%m-%d)\", \"plant_verified\": $verified},"
  { head -n 1 "$report"; printf '%s\n' "$run_line"; tail -n +2 "$report"; } >"$report.tmp"
  mv "$report.tmp" "$report"
fi
cat "$report"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "### plant (exit $rc)"
    echo '```'
    cat "$report"
    echo '```'
  } >>"$GITHUB_STEP_SUMMARY"
fi
exit "$rc"
