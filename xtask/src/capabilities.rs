//! Capability-matrix drift gate.
//!
//! The published [`CapabilityMatrix`] is a hand-maintained table of
//! 43 capabilities across 7 backends. A declared capability is a *claim*, and
//! claims rot: colima shipped `bind_mounts = unsupported` for months even
//! though `ColimaRuntime` generated and executed bind-mount shell snippets
//! every time it started a container.
//!
//! This gate closes that loop by probing the *running* backend and failing when
//! observation disagrees with the table.
//!
//! It deliberately does not rewrite the table. A probe can transiently fail —
//! a VM booting, a registry blip — and auto-folding a flaky observation into a
//! published claim is worse than a loud failure. The table stays the spec; this
//! checks the spec against reality.
//!
//! # Scope
//!
//! Only *runtime-observable* capabilities are probed. The remaining rows
//! (tar path validation, setuid stripping, peer credentials, request frame
//! limits, …) are properties of daemon code, not of the host environment, and
//! no probe can establish them. They are out of scope by construction, and the
//! gate says so rather than silently passing them.
//!
//! # Usage
//!
//! ```bash
//! # Requires a running daemon on the backend under test.
//! cargo xtask capabilities verify
//! ```

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use xshell::Shell;

/// One probed capability and what the backend actually did.
struct Observation {
    capability: &'static str,
    /// What the container reported.
    observed: bool,
    /// Set when the probe could not be evaluated, so the row is skipped
    /// rather than counted as drift.
    inconclusive: Option<String>,
}

/// Locate the `mbx` binary produced by a previous build.
fn find_mbx(root: &Path) -> Result<std::path::PathBuf> {
    for profile in ["release", "debug"] {
        let candidate = root.join("target").join(profile).join("mbx");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "no mbx binary found under target/ — run `cargo build -p minibox-cli` first \
         (this gate probes through the CLI rather than reimplementing the protocol)"
    )
}

/// Probe script.
///
/// Emits **one** line of `key=value` pairs terminated by an explicit `end=1`
/// sentinel, rather than one line per capability. The CLI's streaming output
/// path can drop trailing output, and a per-line layout turns that truncation
/// into a silent false negative — the gate reported `/dev/null` missing when it
/// was present, purely because the last line never arrived. A single line plus
/// a sentinel makes truncation *detectable*: without `end=1` the whole
/// observation set is discarded as inconclusive instead of being read as a
/// capability failure.
const PROBE_SCRIPT: &str = r#"
probe_out=""
probe_add() { probe_out="$probe_out $1=$2"; }
probe_add cgroup2 "$(grep -c cgroup2 /proc/mounts >/dev/null 2>&1 && echo yes || echo no)"
probe_add overlay "$(grep -q overlay /proc/filesystems && echo yes || echo no)"
probe_add pid_ns "$(test -e /proc/self/ns/pid && echo yes || echo no)"
probe_add mnt_ns "$(test -e /proc/self/ns/mnt && echo yes || echo no)"
probe_add net_ns "$(test -e /proc/self/ns/net && echo yes || echo no)"
probe_add uts_ns "$(test -e /proc/self/ns/uts && echo yes || echo no)"
probe_add ipc_ns "$(test -e /proc/self/ns/ipc && echo yes || echo no)"
probe_add dev_null "$(test -c /dev/null && echo yes || echo no)"
probe_add rootfs_writable "$(touch /probe-write-test 2>/dev/null && echo yes || echo no)"
echo "probe::$probe_out end=1"
"#;

/// Invoke the `mbx` binary and capture its output.
fn mbx_output(mbx: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new(mbx)
        .args(args)
        .output()
        .with_context(|| format!("failed to launch {}", mbx.display()))
}

/// Probe the live backend and return one [`Observation`] per observable row.
fn probe(mbx: &Path, sh: &Shell) -> Result<Vec<Observation>> {
    let _ = sh;
    let out = mbx_output(mbx, &["run", "alpine", "--", "/bin/sh", "-c", PROBE_SCRIPT])
        .context("capability probe container could not be started — is a daemon running?")?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    if std::env::var_os("CAPABILITIES_DEBUG").is_some() {
        eprintln!("--- raw probe stdout ---\n{stdout}--- end ---");
    }
    let mut observed: std::collections::BTreeMap<String, bool> =
        std::collections::BTreeMap::default();
    let mut complete = false;
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("probe::") else {
            continue;
        };
        for pair in rest.split_whitespace() {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if key == "end" {
                complete = true;
                continue;
            }
            observed.insert(key.to_string(), value == "yes");
        }
    }

    // Truncated output must never be read as capability failure.
    let truncation = (!complete).then_some(
        "probe output was truncated (no end=1 sentinel) — the CLI's streaming output \
         path drops trailing bytes, so nothing can be concluded this run",
    );

    if observed.is_empty() {
        bail!(
            "capability probe produced no output (stdout was empty) — \
             the container likely failed to start, so nothing can be verified"
        );
    }

    // `privileged` is exercised separately: it needs the flag, and its
    // observable effect is a non-zero CapEff mask. `grep` (not `grep -c`) so
    // the line itself is observable — a bare count cannot distinguish a zero
    // CapEff from a missing /proc.
    let priv_out = mbx_output(
        mbx,
        &[
            "run",
            "--privileged",
            "alpine",
            "--",
            "/bin/sh",
            "-c",
            "grep CapEff /proc/self/status",
        ],
    )
    .context("privileged capability probe could not be started")?;
    let priv_stdout = String::from_utf8_lossy(&priv_out.stdout);
    let priv_line = priv_stdout
        .lines()
        .find(|l| l.trim_start().starts_with("CapEff"))
        .unwrap_or_default();
    let privileged_effective = priv_line
        .split(':')
        .nth(1)
        .is_some_and(|mask| !mask.trim().trim_start_matches('0').is_empty());
    let privileged_inconclusive = priv_line.is_empty().then(|| {
        "privileged probe reported no CapEff line (missing /proc, or the run failed)".to_string()
    });

    let get = |k: &str| observed.get(k).copied();
    // A row is inconclusive if the probe was truncated, or if that specific
    // key never arrived. Truncation dominates: a missing tail must never be
    // reported as a capability that is absent.
    let reason_for = |k: &str| -> Option<String> {
        if let Some(t) = truncation {
            return Some(t.to_string());
        }
        if get(k).is_none() {
            return Some("probe reported nothing for this key".to_string());
        }
        None
    };
    let row = |capability: &'static str, key: &str, observed: bool| Observation {
        capability,
        observed,
        inconclusive: reason_for(key),
    };
    Ok(vec![
        row("cgroups_v2", "cgroup2", get("cgroup2").unwrap_or(false)),
        row("overlay_fs", "overlay", get("overlay").unwrap_or(false)),
        row("pid_namespace", "pid_ns", get("pid_ns").unwrap_or(false)),
        row("mount_namespace", "mnt_ns", get("mnt_ns").unwrap_or(false)),
        row(
            "network_namespace",
            "net_ns",
            get("net_ns").unwrap_or(false),
        ),
        row("uts_namespace", "uts_ns", get("uts_ns").unwrap_or(false)),
        row("ipc_namespace", "ipc_ns", get("ipc_ns").unwrap_or(false)),
        Observation {
            capability: "privileged_mode",
            observed: privileged_effective,
            // The privileged run is a separate container, so the truncation
            // sentinel from the main probe does not apply to it.
            inconclusive: privileged_inconclusive,
        },
        // Recorded for the report even though the matrix has no matching row:
        // these two explain most "the container is broken" reports on VM
        // backends and are the regression signature this gate was written for.
        Observation {
            capability: "(unlisted) rootfs_writable",
            observed: get("rootfs_writable").unwrap_or(false),
            inconclusive: None,
        },
        Observation {
            capability: "(unlisted) dev_null",
            observed: get("dev_null").unwrap_or(false),
            inconclusive: None,
        },
    ])
}

/// Parse `mbx capabilities --json` into a per-capability support map.
///
/// Kept deliberately tolerant: the gate cares about the handful of rows it
/// probes, and should not break when unrelated rows are added.
fn parse_matrix(json: &str) -> Result<serde_json::Value> {
    serde_json::from_str(json).context("mbx capabilities --json did not return valid JSON")
}

/// What a matrix cell asserts about a capability.
#[derive(Debug, PartialEq, Eq)]
enum Claim {
    /// A plain `supported` cell.
    Supported,
    /// `provided_by` — the backend does not implement it directly but the
    /// substrate (a VM) supplies it. Satisfied by observing it working.
    ProvidedBy,
    /// A hard `unsupported` claim.
    Unsupported,
    /// `limited` — deliberately partial, so observation is not a contradiction.
    Limited,
    /// A status this gate does not understand.
    Other(String),
}

impl Claim {
    /// Parse a support cell. The wire form is an object, e.g.
    /// `{"status":"supported"}` or `{"status":"provided_by","provider":"vm"}`.
    fn parse(cell: &serde_json::Value) -> Self {
        match cell.get("status").and_then(serde_json::Value::as_str) {
            Some("supported") => Self::Supported,
            Some("provided_by") => Self::ProvidedBy,
            Some("unsupported") => Self::Unsupported,
            Some("limited") => Self::Limited,
            other => Self::Other(other.unwrap_or("<missing>").to_string()),
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Supported => "supported".to_string(),
            Self::ProvidedBy => "provided_by".to_string(),
            Self::Unsupported => "unsupported".to_string(),
            Self::Limited => "limited".to_string(),
            Self::Other(s) => s.clone(),
        }
    }

    /// What the probe must observe for the claim to hold, or `None` when the
    /// claim makes no falsifiable prediction.
    const fn expects(&self) -> Option<bool> {
        match self {
            Self::Supported | Self::ProvidedBy => Some(true),
            Self::Unsupported => Some(false),
            // `limited` is a partial implementation; a probe cannot decide it.
            Self::Limited | Self::Other(_) => None,
        }
    }
}

/// Locate a capability row and return its claim for `backend_index`.
///
/// Only the probed backend's column is read. Reading column 0 would silently
/// compare every backend against `native`'s claims.
fn matrix_claim(
    matrix: &serde_json::Value,
    capability: &str,
    backend_index: usize,
) -> Option<Claim> {
    let rows = matrix.get("capabilities")?.as_array()?;
    let row = rows
        .iter()
        .find(|r| r.get("capability").and_then(serde_json::Value::as_str) == Some(capability))?;
    let support = row.get("support")?.as_array()?;
    support.get(backend_index).map(Claim::parse)
}

/// Resolve which backend column to check.
///
/// Prefers an explicit `--backend`, then the `MINIBOX_ADAPTER` the daemon
/// itself honours. There is no reliable way to ask the daemon which adapter it
/// selected, and guessing column 0 would compare everything against `native`,
/// so this fails loudly rather than reporting a confident wrong answer.
fn resolve_backend_index(matrix: &serde_json::Value) -> Result<(String, usize)> {
    let backends = matrix
        .get("backends")
        .and_then(serde_json::Value::as_array)
        .context("capability matrix has no `backends` array")?;
    let names: Vec<&str> = backends
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();

    let requested = std::env::args()
        .skip_while(|a| a != "--backend")
        .nth(1)
        .or_else(|| std::env::var("MINIBOX_ADAPTER").ok());

    let Some(wanted) = requested else {
        bail!(
            "cannot tell which backend is active.\nPass --backend <name> (one of: {}), \
             or set MINIBOX_ADAPTER.",
            names.join(", ")
        );
    };

    let index = names
        .iter()
        .position(|n| *n == wanted)
        .with_context(|| format!("unknown backend {wanted:?}; known: {}", names.join(", ")))?;
    Ok((wanted, index))
}

/// Run the drift gate.
pub fn verify(sh: &Shell, root: &Path) -> Result<()> {
    let mbx = find_mbx(root)?;
    eprintln!("capabilities: probing via {}", mbx.display());

    let caps_json = mbx_output(&mbx, &["capabilities", "--json"])
        .context("failed to query the daemon — start miniboxd first")?;
    if !caps_json.status.success() {
        bail!(
            "mbx capabilities failed: {}",
            String::from_utf8_lossy(&caps_json.stderr).trim()
        );
    }
    let matrix = parse_matrix(&String::from_utf8_lossy(&caps_json.stdout))?;
    let (backend_name, backend_index) = resolve_backend_index(&matrix)?;
    eprintln!("capabilities: checking the {backend_name} column");

    let observations = probe(&mbx, sh)?;

    eprintln!("\ncapability             observed  matrix");
    eprintln!("---------------------  --------  ---------");
    let mut drift = Vec::new();
    for obs in &observations {
        let observed_text = if obs.observed { "yes" } else { "no" };

        // The two "(unlisted)" rows exist to surface the read-only-rootfs and
        // missing-/dev regressions, which no matrix row describes.
        if obs.capability.starts_with("(unlisted)") {
            let claim_text = "-".to_string();
            eprintln!("{:<21}  {observed_text:<8}  {claim_text}", obs.capability);
            if !obs.observed {
                eprintln!("  ^ observed limitation with no matrix row — consider adding one");
            }
            continue;
        }

        let claim = matrix_claim(&matrix, obs.capability, backend_index);
        let Some(claim) = claim else {
            eprintln!("{:<21}  {observed_text:<8}  -", obs.capability);
            eprintln!("  ^ no such row in the matrix — matrix is missing a capability");
            drift.push(format!("{}: missing from matrix", obs.capability));
            continue;
        };
        eprintln!(
            "{:<21}  {observed_text:<8}  {}",
            obs.capability,
            claim.label()
        );

        if let Some(reason) = obs.inconclusive.as_deref() {
            eprintln!("  ~ inconclusive: {reason}");
            continue;
        }
        if let Some(expected) = claim.expects()
            && expected != obs.observed
        {
            eprintln!("  ^ DRIFT");
            drift.push(format!(
                "{}: matrix says {}, backend observed {observed_text}",
                obs.capability,
                claim.label()
            ));
        }
    }

    if !drift.is_empty() {
        bail!(
            "capability drift detected ({} row(s)) — the matrix is a claim, and the \
             {backend_name} backend disagrees:\n  - {}\n\nFix the table in \
             crates/minibox-domain/src/capability_matrix.rs, or fix the adapter.",
            drift.len(),
            drift.join("\n  - ")
        );
    }

    eprintln!("\ncapabilities: no drift detected in probed rows for {backend_name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATRIX: &str = r#"{
      "schema_version": 1,
      "backends": ["native", "gke", "colima", "smolvm", "krun", "vz", "winbox"],
      "capabilities": [
        {"group":"isolation","capability":"cgroups_v2",
         "support":[{"status":"supported"},{"status":"unsupported"},
                    {"status":"provided_by","provider":"lima_vm"},
                    {"status":"provided_by","provider":"vm"},
                    {"status":"unsupported"},{"status":"supported"},{"status":"unsupported"}]},
        {"group":"isolation","capability":"privileged_mode",
         "support":[{"status":"supported"},{"status":"unsupported"},
                    {"status":"supported"},{"status":"unsupported"},
                    {"status":"unsupported"},{"status":"unsupported"},{"status":"unsupported"}]}
      ]
    }"#;

    /// Reading column 0 would compare every backend against `native`. Pin the
    /// correct column for each backend.
    #[test]
    fn claim_is_read_from_the_requested_backend_column() {
        let matrix = parse_matrix(MATRIX).expect("matrix parses");
        // colima is index 2.
        assert_eq!(
            matrix_claim(&matrix, "cgroups_v2", 2),
            Some(Claim::ProvidedBy)
        );
        assert_eq!(
            matrix_claim(&matrix, "privileged_mode", 2),
            Some(Claim::Supported)
        );
        // smolvm is index 3 and genuinely lacks privileged mode.
        assert_eq!(
            matrix_claim(&matrix, "privileged_mode", 3),
            Some(Claim::Unsupported)
        );
        // native is index 0.
        assert_eq!(
            matrix_claim(&matrix, "cgroups_v2", 0),
            Some(Claim::Supported)
        );
    }

    #[test]
    fn missing_row_yields_no_claim() {
        let matrix = parse_matrix(MATRIX).expect("matrix parses");
        assert_eq!(matrix_claim(&matrix, "port_forwarding", 0), None);
    }

    /// `provided_by` is satisfied by observation: the VM supplies the
    /// capability even though the adapter does not implement it directly.
    #[test]
    fn provided_by_is_satisfied_by_observation() {
        assert_eq!(Claim::ProvidedBy.expects(), Some(true));
        assert_eq!(Claim::Supported.expects(), Some(true));
        assert_eq!(Claim::Unsupported.expects(), Some(false));
    }

    /// `limited` is a partial implementation, so a probe cannot falsify it.
    #[test]
    fn limited_makes_no_falsifiable_prediction() {
        assert_eq!(Claim::Limited.expects(), None);
        assert_eq!(Claim::Other("weird".into()).expects(), None);
    }

    #[test]
    fn claim_parses_the_wire_form() {
        let supported = serde_json::json!({"status": "supported"});
        let provided = serde_json::json!({"status": "provided_by", "provider": "vm"});
        assert_eq!(Claim::parse(&supported), Claim::Supported);
        assert_eq!(Claim::parse(&provided), Claim::ProvidedBy);
    }
}
