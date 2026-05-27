# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| `main` (pre-release) | Yes — best-effort |
| Released tags | Yes — patch releases for critical CVEs |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Email: **rohi5054@gmail.com**
Subject line: `[SECURITY] Talon — <brief description>`

### What to include

- Affected version(s) / commit hash
- Description of the vulnerability and potential impact
- Steps to reproduce
- Suggested fix (optional but appreciated)

### Response SLA

| Milestone | Target |
|-----------|--------|
| Initial acknowledgment | **48 hours** |
| Severity assessment | **5 business days** |
| Patch + CVE request | **90-day embargo window** |
| Public disclosure | After patch release and embargo window |

### Embargo Policy

We follow a **90-day coordinated disclosure window**:

1. Reporter notifies us privately.
2. We acknowledge and begin remediation.
3. We issue a patch release and request a CVE.
4. After patch is available (or 90 days, whichever is first), the reporter may publish details.

If the vulnerability is already being actively exploited in the wild, we may shorten the window.

### CVE Process

We will request CVEs via the GitHub Security Advisory process for any confirmed vulnerability that affects released versions.

## Supply-Chain Security

Every Talon release binary is:

- **Signed** with `cosign` (keyless, via GitHub OIDC — no stored private key)
- **Attested** with SLSA L2 provenance (`actions/attest-build-provenance`)
- **Checksummed** — `SHA256SUMS` published with every release

To verify a downloaded binary:

```bash
# Verify SHA256
sha256sum --check SHA256SUMS

# Verify cosign signature (requires cosign installed)
cosign verify-blob \
  --certificate-identity-regexp "https://github.com/rohirikman/talon" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  talon
```
