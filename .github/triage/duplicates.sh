#!/usr/bin/env bash
# Point a new issue at the older ones that already report it. Two passes, because every title in
# the repository fits in one request and every body does not: the first picks a shortlist off the
# titles, the second reads those few in full and calls each one the same, related or different,
# and only a clean "same" is posted. The
# model never talks to GitHub, this script does, and it drops any number that was not on the
# shortlist, so the worst a bad answer can do is link the wrong existing issue.
#
#   ISSUE=612 DRY_RUN=1 .github/triage/duplicates.sh
#
# Environment: ISSUE, REPO, GH_TOKEN, TRIAGE_API_KEY, and optionally TRIAGE_BASE_URL,
# TRIAGE_MODEL and DRY_RUN.

set -euo pipefail

tag=duplicates
# shellcheck source=.github/triage/lib.sh
source "$(git rev-parse --show-toplevel)/.github/triage/lib.sh"

marker="<!-- sonora-triage:duplicates -->"

scan="$(yq '.duplicates.scan // 1000' "$config")"
shortlist="$(yq '.duplicates.shortlist // 6' "$config")"
report="$(yq '.duplicates.report // 3' "$config")"

issue_json="$(gh issue view "$issue" --repo "$repo" --json number,title,body,labels,comments,state)"

if [ -z "$dry" ] && [ "$(jq -r '.state' <<<"$issue_json")" != "OPEN" ]; then
  say "issue $issue is closed, nothing to do"
  exit 0
fi

# Said once. An edit that arrives before the first pass found anything gets another look, a
# reopen or a later comment does not. A dry run changes nothing, so it rechecks any issue.
if [ -z "$dry" ] && jq -e --arg marker "$marker" 'any(.comments[]; .body | contains($marker))' <<<"$issue_json" >/dev/null; then
  say "already pointed at something, nothing to do"
  exit 0
fi

# Titles of the issues filed before this one, open and closed alike, newest first. A newer issue
# is never the original, which matters when this is run by hand over an old one. The limit counts
# the newest issues in the whole repository, so once it holds more than `scan` issues the oldest
# ones silently drop out. Raise it in config.yml before that happens.
others="$(gh issue list --repo "$repo" --state all --limit "$scan" \
  --json number,title,state,labels \
  | jq --argjson self "$issue" '[.[] | select(.number < $self)]')"

if [ "$(jq 'length' <<<"$others")" -eq 0 ]; then
  say "no other issues to compare against"
  exit 0
fi

report_of() {
  jq -r \
    '. as $issue |
     "Issue #" + ($issue.number | tostring) +
     " (" + ($issue.state // "OPEN" | ascii_downcase) + ")" +
     "\nLabels: " + ([$issue.labels[].name] | tojson) +
     "\nTitle: " + $issue.title +
     "\nBody:\n" + (($issue.body // "") | .[0:4000])' <<<"$1"
}

new="$(report_of "$issue_json")"

picked="$(decide "$(compose "$root/.github/triage/shortlist.md" \
  "New issue:

$new

Every other issue, newest first:
$(jq -r '.[] | "- #\(.number) (\(.state | ascii_downcase)) \(.title)"' <<<"$others")

Name at most $shortlist.")")"

candidates="$(jq -r \
  --argjson known "$(jq -c 'map(.number)' <<<"$others")" \
  --argjson cap "$shortlist" \
  '[ (.candidates // [])[]
     | select(type == "number")
     | select(. as $n | $known | any(. == $n)) ]
   | unique | .[0:$cap] | .[]' <<<"$picked")"

if [ -z "$candidates" ]; then
  say "nothing on the shortlist"
  exit 0
fi

say "shortlist: $(tr '\n' ' ' <<<"$candidates")"

# The shortlist in full. An issue deleted or turned into a discussion between the list and this
# read is skipped rather than failing the run. Bodies are cut to what the prompt shows.
details='[]'
for number in $candidates; do
  one="$(gh issue view "$number" --repo "$repo" --json number,title,body,state,labels \
    --jq '.body |= ((. // "") | .[0:4000])')" || {
    say "issue $number could not be read, skipping it"
    continue
  }
  details="$(jq --argjson one "$one" '. + [$one]' <<<"$details")"
done

if [ "$(jq 'length' <<<"$details")" -eq 0 ]; then
  say "none of the shortlist could be read"
  exit 0
fi

older=""
while read -r one; do
  older="$older

---

$(report_of "$one")"
done < <(jq -c '.[]' <<<"$details")

verdict="$(decide "$(compose "$root/.github/triage/duplicates.md" \
  "New issue:

$new

Older issues:
$older")")"

say "verdicts: $(jq -c '[(.verdicts // [])[] | {issue, verdict, differs}]' <<<"$verdict")"

# Only a clean "same" is posted. A verdict that names something the new issue has and the older
# one lacks is a partial match, which the model is told to call "related" but may still call same.
found="$(jq -c \
  --argjson details "$details" \
  --argjson cap "$report" \
  '[ (.verdicts // [])[]
     | select(.verdict == "same")
     | select((.differs // "") | type == "string" and (gsub("\\s"; "") | length == 0))
     | select(.issue | type == "number")
     | select(.why | type == "string" and length > 0)
     | select(.issue as $n | $details | any(.number == $n))
     | { issue, why: (.why | .[0:400]) } ]
   | unique_by(.issue) | .[0:$cap]' <<<"$verdict")"

say "note: $(jq -r '.note // ""' <<<"$verdict")"
say "duplicates: $(jq -r 'if length == 0 then "none" else map("#" + (.issue | tostring)) | join(" ") end' <<<"$found")"

if [ "$(jq 'length' <<<"$found")" -eq 0 ]; then
  exit 0
fi

comment="$marker
This may already be reported:

$(jq -r '.[] | "- #\(.issue): \(.why)"' <<<"$found")

If it is the same thing, close this one and follow the older issue for updates. If it is not, leave it open and say so, and a maintainer will take it from here.

> [!NOTE]
> Sonora Buddy is an AI. If it read the issue wrong, say so and a maintainer will look.
"

if [ -n "$dry" ]; then
  say "dry run, commenting on nothing"
  printf '%s\n' "$comment"
  exit 0
fi

gh issue comment "$issue" --repo "$repo" --body "$comment"
