#!/usr/bin/env bash
# Runs plant for the GitHub Action in action.yml. Every input arrives as an
# environment variable; nothing from the event payload is interpolated here.
set -euo pipefail
set -f # no globbing of --only ids

if false; then
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
