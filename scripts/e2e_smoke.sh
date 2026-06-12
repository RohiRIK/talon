#!/usr/bin/env bash
# End-to-end smoke for the Flow Cottage phases (plans/talon-improving-flow-cottage.md).
# Boots a real `talon serve --features web-ui` against an isolated $HOME and
# exercises the live HTTP surface: auth + roles, secrets (headless vault),
# jobs, reliability, webhooks (HMAC + rate limit), metrics, log tail, and
# end-to-end redaction. No LLM key is needed — runs fail at the provider,
# which is itself part of what we assert (failure rows with provenance).
#
# Usage: bash scripts/e2e_smoke.sh [path-to-talon-binary]
set -uo pipefail
cd "$(dirname "$0")/.."

BIN="${1:-target/debug/talon}"
PORT=17791
BASE="http://127.0.0.1:${PORT}"
ADMIN="e2e-admin-token-0123456789"

PASS=0; FAIL=0
ok()   { PASS=$((PASS+1)); echo "  PASS  $1"; }
bad()  { FAIL=$((FAIL+1)); echo "  FAIL  $1"; }
check() { # check <desc> <expected> <actual>
    if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected $2, got $3)"; fi
}

# ── Isolated environment ─────────────────────────────────────────────────────
E2E_HOME="$(mktemp -d)"
export HOME="$E2E_HOME"
mkdir -p "$HOME/.talon"
cat > "$HOME/.talon/config.toml" <<EOF
[gateway]
http_addr = "127.0.0.1:${PORT}"
api_token = "${ADMIN}"

[webhooks]
rate_per_min = 5

[runs]
retention_days = 30
EOF
TALON_MASTER_KEY="$(openssl rand -base64 32)"
export TALON_MASTER_KEY
export TALON_LLM_PROVIDER=anthropic
export TALON_LLM_API_KEY=dummy-key-runs-will-fail-fast
export TALON_SCHEDULER_TICK_SECS=2

SERVE_LOG="$E2E_HOME/serve.log"
"$BIN" serve --gateway http >"$SERVE_LOG" 2>&1 &
SERVE_PID=$!
cleanup() {
    kill "$SERVE_PID" 2>/dev/null
    wait "$SERVE_PID" 2>/dev/null
    rm -rf "$E2E_HOME"
}
trap cleanup EXIT

for _ in $(seq 1 60); do
    code=$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer ${ADMIN}" "$BASE/api/v1/me" || true)
    [ "$code" = "200" ] && break
    sleep 0.5
done
[ "${code:-}" = "200" ] || { echo "FATAL: serve did not come up"; tail -20 "$SERVE_LOG"; exit 1; }
echo "serve is up (pid $SERVE_PID)"

A() { curl -s -H "authorization: Bearer ${ADMIN}" "$@"; }
code_of() { curl -s -o /dev/null -w '%{http_code}' "$@"; }

# ── Auth (criteria 4–6) ──────────────────────────────────────────────────────
check "unauthenticated /api/v1 -> 401" 401 "$(code_of "$BASE/api/v1/jobs")"
check "legacy token is admin via /me" '"admin"' "$(A "$BASE/api/v1/me" | jq .role)"

VIEWER_OUT="$("$BIN" token create e2e-ro --role viewer 2>/dev/null)"
VIEWER="$(echo "$VIEWER_OUT" | grep -o 'talon_[a-f0-9]*' | head -1)"
if [ -n "$VIEWER" ]; then
    check "viewer token reads" 200 "$(code_of -H "authorization: Bearer ${VIEWER}" "$BASE/api/v1/jobs")"
    check "viewer token cannot mutate" 403 "$(code_of -X POST -H "authorization: Bearer ${VIEWER}" -H 'content-type: application/json' -d '{"prompt":"x","schedule":"daily"}' "$BASE/api/v1/jobs")"
else
    bad "token CLI produced no token: $VIEWER_OUT"
fi

# ── Secrets, headless vault (criteria 2, 9, 15) ──────────────────────────────
SECRET_VALUE="super-e2e-secret-value-94137"
check "store secret via API" 201 "$(code_of -X POST -H "authorization: Bearer ${ADMIN}" -H 'content-type: application/json' -d "{\"name\":\"E2E_KEY\",\"value\":\"${SECRET_VALUE}\"}" "$BASE/api/v1/secrets")"
LIST="$(A "$BASE/api/v1/secrets")"
echo "$LIST" | grep -q '"E2E_KEY"' && ok "secret listed by name" || bad "secret missing from list: $LIST"
echo "$LIST" | grep -q "$SECRET_VALUE" && bad "SECRET VALUE LEAKED IN LIST" || ok "list carries no value"
"$BIN" secret list 2>/dev/null | grep -q E2E_KEY && ok "secret CLI works headless (env master key)" || bad "secret CLI headless path"

# ── Jobs + reliability (criteria 21, 28, 29) ─────────────────────────────────
JOB="$(A -X POST -H 'content-type: application/json' -d '{"prompt":"use {{secret:E2E_KEY}} to do nothing","schedule":"0 0 * * *","name":"e2e"}' "$BASE/api/v1/jobs")"
JOB_ID="$(echo "$JOB" | jq -r .id)"
[ -n "$JOB_ID" ] && [ "$JOB_ID" != "null" ] && ok "job created" || bad "job create: $JOB"
check "retry policy patch" 200 "$(code_of -X PATCH -H "authorization: Bearer ${ADMIN}" -H 'content-type: application/json' -d '{"retry_max":1}' "$BASE/api/v1/jobs/$JOB_ID")"
check "self on_failure rejected" 422 "$(code_of -X PATCH -H "authorization: Bearer ${ADMIN}" -H 'content-type: application/json' -d "{\"on_failure\":\"$JOB_ID\"}" "$BASE/api/v1/jobs/$JOB_ID")"

# ── Webhooks (criteria 25–27) ────────────────────────────────────────────────
HOOK="$(A -X POST "$BASE/api/v1/jobs/$JOB_ID/hooks")"
HOOK_ID="$(echo "$HOOK" | jq -r .hook_id)"
HOOK_SECRET="$(echo "$HOOK" | jq -r .secret)"
[ -n "$HOOK_ID" ] && [ "$HOOK_ID" != "null" ] && ok "hook created (secret shown once)" || bad "hook create: $HOOK"

BODY='{"event":"e2e-push"}'
TS="$(date +%s)"
SIG="$(printf '%s.%s' "$TS" "$BODY" | openssl dgst -sha256 -hmac "$HOOK_SECRET" -hex | awk '{print $NF}')"
check "signed delivery -> 202" 202 "$(code_of -X POST -H "x-talon-timestamp: $TS" -H "x-talon-signature: $SIG" -H 'content-type: application/json' -d "$BODY" "$BASE/hooks/$HOOK_ID")"
check "tampered signature -> 401" 401 "$(code_of -X POST -H "x-talon-timestamp: $TS" -H "x-talon-signature: deadbeef$SIG" -H 'content-type: application/json' -d "$BODY" "$BASE/hooks/$HOOK_ID")"

LIMITED=0
for _ in $(seq 1 6); do
    c="$(code_of -X POST -H "x-talon-timestamp: $TS" -H "x-talon-signature: $SIG" -H 'content-type: application/json' -d "$BODY" "$BASE/hooks/$HOOK_ID")"
    [ "$c" = "429" ] && LIMITED=1 && break
done
[ "$LIMITED" = "1" ] && ok "per-hook rate limit -> 429" || bad "rate limit never engaged"

# ── Observability (criteria 16, 19, 20) ──────────────────────────────────────
check "/metrics unauthenticated -> 401" 401 "$(code_of "$BASE/metrics")"
A "$BASE/metrics" | grep -q 'talon_' && ok "/metrics renders talon_ series" || bad "/metrics payload"
TAIL="$(curl -s -N --max-time 3 "$BASE/api/v1/logs/tail?token=${ADMIN}&level=info" | head -1)"
[ -n "$TAIL" ] && ok "log tail streams" || bad "log tail empty"
ls "$HOME/.talon/logs/" | grep -q talon.log && ok "JSON file sink rotating" || bad "no log file"

# ── Run provenance + end-to-end redaction (criteria 10, 26, 30) ──────────────
RUNS=""
for _ in $(seq 1 45); do
    RUNS="$(A "$BASE/api/v1/jobs/$JOB_ID/runs")"
    [ "$(echo "$RUNS" | jq 'length')" -ge 1 ] 2>/dev/null && break
    sleep 1
done
if [ "$(echo "$RUNS" | jq 'length')" -ge 1 ] 2>/dev/null; then
    ok "webhook delivery produced run rows"
    echo "$RUNS" | jq -r '.[].fired_by' | grep -q webhook && ok "run provenance fired_by=webhook" || bad "no webhook provenance: $(echo "$RUNS" | jq -c '[.[].fired_by]')"
    echo "$RUNS" | grep -q "$SECRET_VALUE" && bad "SECRET VALUE LEAKED IN RUN RECORDS" || ok "run records carry no secret value"
    grep -q "$SECRET_VALUE" "$SERVE_LOG" && bad "SECRET VALUE LEAKED IN SERVER LOG" || ok "server log carries no secret value"
else
    bad "no run rows appeared: $RUNS"
fi

echo
echo "e2e: $PASS passed, $FAIL failed"
[ "$FAIL" = "0" ]
