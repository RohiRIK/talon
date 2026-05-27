#!/usr/bin/env bash
# Talon installer — verifies SHA256 checksum and cosign signature before installing.
# Usage: curl -fsSL https://talon.sh/install | sh
#
# TRUST it. It's RUST.
set -euo pipefail

REPO="rohirikman/talon"
INSTALL_DIR="${TALON_INSTALL_DIR:-$HOME/.cargo/bin}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# ── Detect platform ───────────────────────────────────────────────────────────
detect_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64)  echo "x86_64-unknown-linux-gnu" ;;
                aarch64) echo "aarch64-unknown-linux-gnu" ;;
                *) echo "Unsupported Linux architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                x86_64)  echo "x86_64-apple-darwin" ;;
                arm64)   echo "aarch64-apple-darwin" ;;
                *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac
}

# ── Fetch latest release tag from GitHub API ─────────────────────────────────
latest_tag() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
}

# ── Verify SHA256 ─────────────────────────────────────────────────────────────
verify_sha256() {
    local file="$1" expected="$2"
    local actual
    if command -v sha256sum &>/dev/null; then
        actual="$(sha256sum "$file" | awk '{print $1}')"
    elif command -v shasum &>/dev/null; then
        actual="$(shasum -a 256 "$file" | awk '{print $1}')"
    else
        echo "Neither sha256sum nor shasum found — cannot verify checksum" >&2
        exit 1
    fi

    if [ "$actual" != "$expected" ]; then
        echo "SHA256 mismatch!" >&2
        echo "  expected: $expected" >&2
        echo "  actual:   $actual" >&2
        exit 1
    fi
    echo "  SHA256 OK"
}

# ── Verify cosign signature ───────────────────────────────────────────────────
verify_cosign() {
    local file="$1" sig="$2" cert="$3"
    if ! command -v cosign &>/dev/null; then
        echo "  cosign not installed — skipping signature verification"
        echo "  Install cosign: https://docs.sigstore.dev/cosign/system_requirements/"
        return 0
    fi

    cosign verify-blob \
        --certificate-identity-regexp "https://github.com/${REPO}" \
        --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
        --signature "$sig" \
        --certificate "$cert" \
        "$file"
    echo "  cosign OK"
}

main() {
    local target tag base_url binary_name binary sig cert sums expected_sha

    target="$(detect_target)"
    tag="${TALON_VERSION:-$(latest_tag)}"

    echo "Installing talon ${tag} for ${target}..."

    base_url="https://github.com/${REPO}/releases/download/${tag}"
    binary_name="talon"
    binary="${TMP_DIR}/${binary_name}"
    sig="${binary}.sig"
    cert="${binary}.pem"
    sums="${TMP_DIR}/SHA256SUMS"

    echo "Downloading..."
    curl -fsSL "${base_url}/talon-${target}"    -o "$binary"
    curl -fsSL "${base_url}/talon-${target}.sig" -o "$sig"
    curl -fsSL "${base_url}/talon-${target}.pem" -o "$cert"
    curl -fsSL "${base_url}/SHA256SUMS"          -o "$sums"

    echo "Verifying..."
    expected_sha="$(grep "talon-${target}" "$sums" | awk '{print $1}')"
    verify_sha256 "$binary" "$expected_sha"
    verify_cosign "$binary" "$sig" "$cert"

    echo "Installing to ${INSTALL_DIR}..."
    mkdir -p "$INSTALL_DIR"
    chmod +x "$binary"
    mv "$binary" "${INSTALL_DIR}/${binary_name}"

    echo ""
    echo "TRUST it. It's RUST."
    echo "talon ${tag} installed to ${INSTALL_DIR}/talon"
    echo ""
    echo "Run: talon init"
}

main "$@"
