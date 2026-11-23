# shellcheck shell=bash disable=SC2034
# Shared plumbing for the triage scripts: where the config is, how to reach the model, and how
# to turn one answer back into json. Sourced, never run. A script sets `tag` before sourcing
# this to name itself in the log.

root="$(git rev-parse --show-toplevel)"
config="$root/.github/triage/config.yml"

repo="${REPO:-${GITHUB_REPOSITORY:-}}"
issue="${ISSUE:?the issue number is required}"
base="${TRIAGE_BASE_URL:-https://opencode.ai/zen/go/v1}"
model="${TRIAGE_MODEL:-glm-5.3}"
dry="${DRY_RUN:-}"
session="triage-${repo//\//-}-$issue"

: "${TRIAGE_API_KEY:?the inference api key is required}"

say() { printf '%s: %s\n' "${tag:-triage}" "$*" >&2; }

# opencode Go routes on the session id and refuses a request without one, so every call about
# the same issue carries the same one and shares its prompt cache. The request goes in on stdin
# because a prompt holding every title in the repository outgrows a single argument.
ask() {
  curl -sS --fail-with-body --retry 2 --retry-all-errors --max-time 120 \
    -H "Authorization: Bearer $TRIAGE_API_KEY" \
    -H 'Content-Type: application/json' \
    -H "x-opencode-session: $session" \
    -A "sonora-triage/1.0" \
    --data-binary @- \
    "$base/chat/completions" <<<"$1"
}

# One system prompt file and one user message, turned into a chat request.
compose() {
  jq -n --arg model "$model" --rawfile system "$1" --rawfile user <(printf '%s' "$2") \
    '{
       model: $model,
       temperature: 0,
       response_format: { type: "json_object" },
       messages: [
         { role: "system", content: $system },
         { role: "user", content: $user }
       ]
     }'
}

# Ask, and print the object the model answered with. Not every OpenAI compatible endpoint takes
# response_format, so a refusal means asking again without it and leaning on the prompt for the
# shape, and a code fence the model wrapped the object in anyway is stripped.
decide() {
  local answer content
  answer="$(ask "$1")" || {
    say "the endpoint refused json mode, asking again without it"
    answer="$(ask "$(jq 'del(.response_format)' <<<"$1")")"
  }

  content="$(jq -r '.choices[0].message.content // empty' <<<"$answer")"
  if [ -z "$content" ]; then
    say "the model returned nothing usable"
    jq -c '.' <<<"$answer" >&2
    return 1
  fi

  sed -e 's/^```json//' -e 's/^```//' -e 's/```$//' <<<"$content" | jq '.'
}
