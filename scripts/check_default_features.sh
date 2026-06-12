#!/usr/bin/env bash
# Criterion 33 (partial): default builds stay lean — external secret
# providers and OTel are compile-time features. Fails when their deps leak
# into the default dependency tree.
set -euo pipefail
cd "$(dirname "$0")/.."

tree="$(cargo tree -e normal --quiet)"
for dep in aws-sdk-secretsmanager aws-config opentelemetry wiremock; do
    if grep -Eq "\b${dep} v[0-9]" <<<"$tree"; then
        echo "FAIL: ${dep} present in default dependency tree" >&2
        exit 1
    fi
done

# talon-secrets itself must not pull reqwest by default (vault feature only;
# reqwest elsewhere in the workspace is fine).
if cargo tree -p talon-secrets -e normal --quiet | grep -Eq "\breqwest v[0-9]"; then
    echo "FAIL: reqwest in talon-secrets default tree (vault feature leak)" >&2
    exit 1
fi

echo "OK: default build excludes vault/aws-secrets/otel deps"
