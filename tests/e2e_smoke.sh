#!/usr/bin/env bash

set -euo pipefail

BASE_URL="${AYATORI_BASE_URL:-http://localhost:8080}"
AUTH_HEADER=()
if [[ -n "${AYATORI_API_KEY:-}" ]]; then
  AUTH_HEADER=(-H "Authorization: Bearer ${AYATORI_API_KEY}")
fi

OPENAI_MODEL="${OPENAI_RESPONSES_MODEL:-}"
ANTHROPIC_MODEL="${ANTHROPIC_RESPONSES_MODEL:-}"
VERTEX_MODEL="${VERTEX_RESPONSES_MODEL:-}"
OLLAMA_MODEL="${OLLAMA_RESPONSES_MODEL:-}"

curl_json() {
  local path="$1"
  local payload="$2"
  curl --fail --silent --show-error \
    "${AUTH_HEADER[@]}" \
    -H "Content-Type: application/json" \
    -X POST "${BASE_URL}${path}" \
    -d "${payload}"
}

echo "== health =="
curl --fail --silent --show-error "${BASE_URL}/healthz"
echo

if [[ -n "${OPENAI_MODEL}" ]]; then
  echo "== openai basic =="
  curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OPENAI_MODEL}","input":"hello from smoke test"}
JSON
)"
  echo

  echo "== openai structured output =="
  curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OPENAI_MODEL}","input":"return Tokyo as JSON","text":{"format":{"type":"json_schema","name":"city_response","schema":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]},"strict":true}}}
JSON
)"
  echo
fi

if [[ -n "${ANTHROPIC_MODEL}" ]]; then
  echo "== anthropic streaming =="
  curl --fail --silent --show-error -N \
    "${AUTH_HEADER[@]}" \
    -H "Content-Type: application/json" \
    -X POST "${BASE_URL}/v1/responses" \
    -d "{\"model\":\"${ANTHROPIC_MODEL}\",\"input\":\"count to five\",\"stream\":true}"
  echo
fi

if [[ -n "${VERTEX_MODEL}" ]]; then
  echo "== vertex function calling =="
  curl_json "/v1/responses" "$(cat <<JSON
{"model":"${VERTEX_MODEL}","input":"what is the weather?","tools":[{"type":"function","name":"lookup_weather","description":"Lookup the weather","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}],"tool_choice":"required"}
JSON
)"
  echo
fi

if [[ -n "${OLLAMA_MODEL}" ]]; then
  echo "== ollama image input =="
  curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OLLAMA_MODEL}","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"describe this image"},{"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"low"}]}]}
JSON
)"
  echo
fi

if [[ -n "${OPENAI_MODEL}" ]]; then
  echo "== previous_response_id chain =="
  first_response="$(curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OPENAI_MODEL}","input":"remember that my city is Tokyo"}
JSON
)")"
  echo "${first_response}"
  response_id="$(printf '%s' "${first_response}" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
  curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OPENAI_MODEL}","input":"what city did I mention?","previous_response_id":"${response_id}"}
JSON
)"
  echo

  echo "== background lifecycle =="
  background_response="$(curl_json "/v1/responses" "$(cat <<JSON
{"model":"${OPENAI_MODEL}","input":"say hello later","background":true}
JSON
)")"
  echo "${background_response}"
fi
