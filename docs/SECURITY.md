# Security Policy

## Overview

Minibox is a Docker-like container runtime with security as a core design principle. This document outlines the threat model, security architecture, vulnerability disclosure process, and security testing strategy.

**Security Status:** Suitable for controlled internal deployments with monitoring
**Last Security Audit:** 2026-03-15
**Fixed Vulnerabilities:** 12/15 critical/high severity (see SECURITY_FIXES.md)

## Reporting Security Vulnerabilities

### Disclosure Process

We take security vulnerabilities seriously. If you discover a security issue:

1. **DO NOT** open a public GitHub issue
2. Email security reports to: [Your security contact email]
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if applicable)

### Response Timeline

- **Acknowledgment:** Within 48 hours
- **Initial Assessment:** Within 1 week
- **Fix Development:** Based on severity (critical: 48-72 hours, high: 1-2 weeks)
- **Public Disclosure:** After fix is released and users have time to upgrade (typically 2-4 weeks)

### Severity Classification

We use CVSS 3.1 scoring:

- **Critical (9.0-10.0):** Container escape, arbitrary code execution, privilege escalation
- **High (7.0-8.9):** DoS attacks, information disclosure, authentication bypass
- **Medium (4.0-6.9):** Resource exhaustion, input validation issues
- **Low (0.1-3.9):** Information leakage, minor security issues

## Threat Model

### Trust Boundaries

```
┌─────────────────────────────────────────────────────────────┐
│                         Host System                          │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                    Trusted Zone                        │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │          Minibox Daemon (Root Process)          │  │  │
│  │  │  - Unix socket authentication (SO_PEERCRED)     │  │  │
│  │  │  - Request validation and rate limiting         │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  │                           ▲                            │  │
│  │                           │ Authenticated              │  │
│  │                           │ JSON Protocol              │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │          Minibox CLI (Root Process)             │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                   Untrusted Zone                       │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │            Container (Isolated Process)         │  │  │
│  │  │  - Linux namespaces (PID, Mount, UTS, IPC, Net) │  │  │
│  │  │  - cgroups v2 resource limits                   │  │  │
│  │  │  - Secure mount flags (nosuid, nodev, noexec)   │  │  │
│  │  │  - Read-only /sys                               │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌───────────────────────────────────────────────────────┐  │
│  │              External Zone (Internet)                  │  │
│  │  - Docker Hub registry (HTTPS-only, TLS 1.2+)         │  │
│  │  - Size limits enforced (1 GB/layer, 10 MB manifest) │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Attack Surfaces

#### 1. Unix Socket API (Daemon)

**Attack Vectors:**

- Malicious JSON requests
- Resource exhaustion via concurrent requests
- Authentication bypass attempts
- Request size DoS

**Mitigations:**

- SO_PEERCRED authentication (root-only)
- 1 MB request size limit
- Concurrent spawn semaphore (100 max)
- Socket permissions 0600

**Residual Risks:**

- Request rate limiting not yet implemented
- No request throttling per client

#### 2. mbxctl HTTP API (Control Plane)

**Attack Vectors:**

- Unauthenticated job creation — any process with network access can run containers
- Request body DoS

**Mitigations:**

- 1 MB request body limit (`DefaultBodyLimit`)
- Systemd unit binds `localhost:9999` by default (not 0.0.0.0)

**Known Limitation — No Authentication:**

`mbxctl` exposes an **unauthenticated** HTTP API. Anyone with network access to the
listen address can create, query, and delete container jobs. This is intentional for
local/trusted-network deployments but must be addressed before any internet-facing use.

Recommended mitigations for production deployments:

- Keep the default `localhost:9999` bind and access via SSH tunnel
- Add a reverse proxy with mTLS or shared-secret header (e.g. `X-API-Key`)
- Firewall the port if binding to a non-loopback interface

**Residual Risks:**

- No authentication mechanism built in
- No rate limiting per client

#### 3. Container Runtime

**Attack Vectors:**

- Container escape via namespace violations
- Privilege escalation via setuid binaries
- Fork bombs
- Resource exhaustion (memory, CPU, disk I/O)

**Mitigations:**

- Linux namespaces (PID, Mount, UTS, IPC, Network)
- cgroups v2 limits (memory, CPU weight, PID max, I/O throttling)
- Secure mount flags (MS_NOSUID, MS_NODEV, MS_NOEXEC)
- Read-only /sys mount

**Residual Risks:**

- No capability dropping (running with full root capabilities)
- No seccomp filters
- No user namespace remapping

#### 3. Image Registry

**Attack Vectors:**

- Man-in-the-middle attacks
- Malicious image layers
- Zip Slip attacks via crafted tar archives
- Disk exhaustion via large images

**Mitigations:**

- HTTPS-only connections with TLS 1.2+
- 1 GB per-layer size limit
- 10 MB manifest size limit
- Tar entry validation (no .., symlinks, device nodes)
- Setuid/setgid bit stripping

**Residual Risks:**

- No image signature verification
- No content trust/Notary support

#### 4. Filesystem Operations

**Attack Vectors:**

- Path traversal via malicious image names/tags
- Symlink attacks
- Directory permission violations

**Mitigations:**

- Path canonicalization with .. rejection
- Symlink validation
- 0700 permissions on container directories
- Overlay filesystem path validation

**Residual Risks:**

- No filesystem quota enforcement

### Threat Actors

#### 1. Malicious Container User

**Capabilities:**

- Can execute arbitrary code inside container
- Has root privileges inside container namespace

**Goals:**

- Escape container to host
- Escalate privileges on host
- Access other containers' data

**Mitigations:**

- Namespace isolation
- cgroups resource limits
- Secure mount flags
- Path validation

#### 2. Malicious Registry Provider

**Capabilities:**

- Control over image layers and manifests
- Can craft malicious tar archives

**Goals:**

- Execute code on host during image pull
- Exfiltrate host data
- Corrupt host filesystem

**Mitigations:**

- HTTPS enforcement
- Tar entry validation
- Size limits
- Path validation

#### 3. Local Unprivileged User

**Capabilities:**

- Can attempt Unix socket connections
- Can read world-readable files

**Goals:**

- Control daemon without authorization
- Access container data
- Cause denial of service

**Mitigations:**

- SO_PEERCRED authentication
- Socket permissions 0600
- Container directory permissions 0700

## Security Architecture

### Defense in Depth Layers

**Layer 1: Input Validation**

- Request size limits (1 MB)
- Image size limits (1 GB/layer)
- Path canonicalization
- Tar entry validation
- Range validation for cgroup values

**Layer 2: Authentication & Authorization**

- SO_PEERCRED Unix socket authentication
- Root-only daemon access (UID 0)
- Socket permissions enforcement

**Layer 3: Isolation**

- Linux namespaces (PID, Mount, UTS, IPC, Network)
- cgroups v2 resource limits
- Overlay filesystem with secure mount flags
- Read-only /sys mount

**Layer 4: Resource Limits**

- Memory limits (configurable)
- CPU weight (configurable)
- PID limits (1024 default)
- I/O bandwidth throttling
- Concurrent spawn semaphore (100)

**Layer 5: Network Security**

- HTTPS-only registry connections
- TLS 1.2+ minimum
- Isolated network namespace per container

### Security-Critical Code Paths

#### Path Validation (filesystem.rs)

```rust
pub fn validate_layer_path(base: &Path, layer: &Path) -> Result<PathBuf>
```

**Purpose:** Prevent path traversal attacks in overlay filesystem
**Validation:**

- Canonicalizes paths
- Rejects .. components
- Ensures paths stay within base directory

**Testing:** Unit tests required

#### Tar Entry Validation (layer.rs)

```rust
// Manual entry validation in extract_layer()
```

**Purpose:** Prevent Zip Slip attacks during image extraction
**Validation:**

- Rejects absolute symlinks
- Rejects .. components in paths
- Rejects device nodes, pipes
- Strips setuid/setgid bits

**Testing:** Integration tests required

#### Unix Socket Authentication (server.rs)

```rust
fn authenticate_client(stream: &UnixStream) -> Result<(u32, u32)>
```

**Purpose:** Ensure only root can control daemon
**Validation:**

- SO_PEERCRED credential retrieval
- UID 0 check
- Audit logging of client PID/UID

**Testing:** Integration tests required

## Security Testing Strategy

### Unit Tests

**Coverage Requirements:**

- 100% coverage of security-critical functions
- Path validation edge cases
- Tar entry validation edge cases
- Authentication logic

**Test Cases:**

- Valid and invalid paths
- Symlink attacks
- Path traversal attempts
- Boundary conditions

### Integration Tests

**Required Tests:**

- Real tar archives with malicious entries
- Docker Hub image pull with size enforcement
- cgroups limit enforcement
- Authentication bypass attempts
- Concurrent spawn limits

**Current Status:** 24+ integration tests implemented (cgroup + handler), security tests pending

### Security Tests

**Planned Tests:**

- Container escape attempts
- Privilege escalation attempts
- Fork bomb resistance
- DoS attack resistance
- Network isolation verification

**Tools:**

- Custom test harness
- Fuzzing with cargo-fuzz
- Static analysis with cargo-clippy

### Penetration Testing

**Scope:**

- Container escape attempts
- Privilege escalation
- Authentication bypass
- Resource exhaustion attacks

**Timeline:** Before production deployment

## Continuous Security Monitoring

### Automated Scanning

**Dependency Vulnerabilities:**

- Tool: cargo-deny
- Frequency: On every commit (GitHub Actions)
- Action: Block PRs with critical vulnerabilities

**Static Analysis:**

- Tool: cargo-clippy with security lints
- Frequency: Pre-commit hook + CI
- Action: Enforce zero warnings

**Code Scanning:**

- Tool: GitHub Advanced Security (optional)
- Frequency: On every push
- Action: Review alerts

### Manual Reviews

**Security-Focused Code Review:**

- All changes to security-critical paths require review
- Focus areas:
  - Input validation
  - Authentication logic
  - Resource limits
  - Filesystem operations

**Periodic Security Audits:**

- Frequency: Quarterly
- Scope: Full codebase review
- Focus: New attack vectors, evolving threats

## Security Hardening Roadmap

### Phase 1: Critical Vulnerabilities (Completed)

- [FIXED] Path traversal prevention
- [FIXED] Tar extraction safety
- [FIXED] Unix socket authentication
- [FIXED] Image size limits
- [FIXED] cgroups PID/IO limits
- [FIXED] Secure mount flags
- [FIXED] ImageStore path validation
- [FIXED] HTTPS enforcement
- [FIXED] Directory permissions
- [FIXED] Concurrent spawn limits
- [FIXED] Request size limits
- [FIXED] Container ID collision prevention

### Phase 2: High-Priority Hardening (Next 2-3 weeks)

- [ ] Capability dropping (CAP_SYS_ADMIN, CAP_SYS_MODULE, etc.)
- [ ] Seccomp filters (block dangerous syscalls)
- [ ] Request rate limiting (token bucket)
- [ ] Comprehensive security test suite

### Phase 3: Advanced Security (Future)

- [ ] User namespace support
- [ ] Image signature verification (Sigstore/Cosign)
- [ ] Audit logging to persistent storage
- [ ] SELinux/AppArmor integration
- [ ] Network policy enforcement

## Compliance Considerations

### SOC2 Type II

**Relevant Controls:**

- CC6.1: Access controls implemented (Unix socket auth)
- CC6.2: Data protection via directory permissions
- CC7.2: Security event logging
- CC7.3: Input validation for threat detection

### PCI-DSS (if applicable)

**Relevant Requirements:**

- Req 2: Secure defaults partially implemented
- Req 8: Authentication implemented (root-only)
- Req 10: Basic logging present

**Note:** Minibox is not currently suitable for processing payment data without additional hardening.

## Security Best Practices

### For Users

1. **Run only trusted images** - Verify image sources
2. **Set resource limits** - Always specify memory/CPU limits
3. **Monitor logs** - Watch for unusual activity
4. **Keep updated** - Apply security patches promptly
5. **Isolate network** - Use firewall rules for container network
6. **Limit daemon access** - Only trusted root users should access daemon

### For Developers

1. **Security-first design** - Consider security impact of all changes
2. **Input validation** - Validate all external inputs
3. **Fail securely** - Default to deny on errors
4. **Minimal privileges** - Request only necessary capabilities
5. **Defense in depth** - Layer security controls
6. **Document assumptions** - Security-critical code needs clear documentation

## References

- [SECURITY_FIXES.md](SECURITY_FIXES.md) - Detailed vulnerability fixes
- [TESTING.md](TESTING.md) - Testing strategy
- [CLAUDE.md](CLAUDE.md) - Development guide
- OWASP Container Security: https://cheatsheetseries.owasp.org/cheatsheets/Docker_Security_Cheat_Sheet.html
- CIS Docker Benchmark: https://www.cisecurity.org/benchmark/docker
- NIST 800-190: Application Container Security Guide

## Changelog

### 2026-03-16

- Created comprehensive security policy
- Documented threat model and attack surfaces
- Established security testing strategy
- Defined continuous monitoring approach

### 2026-03-15

- Fixed 12 critical/high severity vulnerabilities
- Implemented input validation and authentication
- Added resource limits and isolation controls
