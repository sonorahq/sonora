#!/usr/bin/env bash
# Reruns the failed jobs of run $RUN_ID in $GH_REPO when every one of them is a job GitHub
# cancelled for want of a hosted runner. Such a job never ran a step, so running it again is all a
# person would do. A run that also failed for real is left alone, since its fix brings a new run,
# and so is the run of a pull request that has since closed. Only the newest run of its workflow
# in its concurrency group is rerun: an older one would rejoin the group and cancel or replace the
# run that superseded it. DRY_RUN=1 only reports.
set -euo pipefail

# A run is tried three times in all, so a pool that stays empty does not keep it going all day.
max_attempts=3

# One field per line, so a missing branch stays an empty line instead of shifting the others.
run=$(
  gh api "repos/$GH_REPO/actions/runs/$RUN_ID" \
    -q '.workflow_id, .head_branch, .event, .head_sha, .run_attempt'
)
{ read -r workflow; read -r branch; read -r event; read -r sha; read -r attempt; } <<<"$run"
if (( attempt >= max_attempts )); then
  echo "rerun: run $RUN_ID is on attempt $attempt already, leaving it"
  exit 0
fi

# The run's own list of pull requests is empty for a fork, so the open pull request is found by
# its head branch and commit instead.
if [[ $event == pull_request ]]; then
  open=$(
    gh pr list --state open --head "$branch" --json headRefOid \
      -q ".[] | select(.headRefOid == \"$sha\") | .headRefOid"
  )
  if [[ -z $open ]]; then
    echo "rerun: the pull request of run $RUN_ID is closed, leaving it"
    exit 0
  fi
fi

jobs=$(
  gh api --paginate "repos/$GH_REPO/actions/runs/$RUN_ID/jobs?filter=latest&per_page=100" \
    -q '.jobs[] | select(.conclusion == "failure" or .conclusion == "cancelled") | .id'
)
if [[ -z $jobs ]]; then
  echo "rerun: no job of run $RUN_ID failed"
  exit 0
fi
for job in $jobs; do
  annotations=$(gh api "repos/$GH_REPO/check-runs/$job/annotations" -q '.[].message')
  if ! grep -q 'was not acquired by Runner' <<<"$annotations"; then
    echo "rerun: job $job of run $RUN_ID failed for real, leaving the run"
    exit 0
  fi
done

# Checked last, so a push that lands while the rest is looked up is still seen. A pull request's
# runs and a branch's own runs never share a concurrency group, so each is compared with its kind.
case $event in pull_request) pr=true ;; *) pr=false ;; esac
newest=$(
  gh api -X GET "repos/$GH_REPO/actions/workflows/$workflow/runs" -f branch="$branch" -F per_page=30 \
    -q "[.workflow_runs[] | select((.event == \"pull_request\") == $pr)][0].id"
)
if [[ $newest != "$RUN_ID" ]]; then
  echo "rerun: run $newest on $branch superseded run $RUN_ID, leaving it"
  exit 0
fi

echo "rerun: no runner took the failed jobs of run $RUN_ID ($(wc -w <<<"$jobs")), rerunning them"
if [[ -z ${DRY_RUN:-} ]]; then
  gh run rerun "$RUN_ID" --failed
fi
