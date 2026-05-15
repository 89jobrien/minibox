use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, IsTerminal as _, Read},
    path::{Path, PathBuf},
};

const LOCK_PATH: &str = "xtask/protocol-drift.lock";
const ALGORITHM: &str = "sha256-normalized-core-contract-v1";

const SURFACES: &[Surface] = &[
    Surface {
        name: "wire-protocol",
        path: "crates/minibox-core/src/protocol.rs",
    },
    Surface {
        name: "domain-ports",
        path: "crates/minibox-core/src/domain.rs",
    },
    Surface {
        name: "domain-networking",
        path: "crates/minibox-core/src/domain/networking.rs",
    },
    Surface {
        name: "domain-extensions",
        path: "crates/minibox-core/src/domain/extensions.rs",
    },
    Surface {
        name: "lifecycle-events",
        path: "crates/minibox-core/src/events.rs",
    },
    Surface {
        name: "execution-manifest",
        path: "crates/minibox-core/src/domain/execution_manifest.rs",
    },
    Surface {
        name: "execution-policy",
        path: "crates/minibox-core/src/domain/execution_policy.rs",
    },
    Surface {
        name: "error-types",
        path: "crates/minibox-core/src/error.rs",
    },
    Surface {
        name: "client-api",
        path: "crates/minibox-core/src/client/mod.rs",
    },
    Surface {
        name: "typestate",
        path: "crates/minibox-core/src/typestate.rs",
    },
];

#[derive(Debug, Clone, Copy)]
struct Surface {
    name: &'static str,
    path: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Lockfile {
    version: u8,
    algorithm: String,
    surfaces: Vec<SurfaceHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SurfaceHash {
    name: String,
    path: String,
    hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Drift {
    name: String,
    path: String,
    expected: Option<String>,
    actual: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HookInput {
    tool_input: Option<HookToolInput>,
}

#[derive(Debug, Deserialize)]
struct HookToolInput {
    file_path: Option<PathBuf>,
}

pub fn run(root: &Path, update: bool, warn_only: bool, hook: bool) -> Result<()> {
    let hook_path = if hook { read_hook_file_path()? } else { None };

    if let Some(path) = &hook_path {
        if !is_tracked_surface_path(root, path) {
            return Ok(());
        }
        eprintln!(
            "[protocol-drift] {} is a hash-tracked core contract surface",
            display_relative(root, path).display()
        );
    }

    let current = calculate_lockfile(root)?;
    let lock_path = root.join(LOCK_PATH);

    if update {
        write_lockfile(&lock_path, &current)?;
        eprintln!(
            "protocol-drift: updated {} with {} surface hash(es)",
            LOCK_PATH,
            current.surfaces.len()
        );
        return Ok(());
    }

    let expected = read_lockfile(&lock_path)?;
    let drift = compare_lockfiles(&expected, &current);

    if drift.is_empty() {
        if !hook {
            eprintln!(
                "protocol-drift: OK ({} core contract surface hash(es) match)",
                current.surfaces.len()
            );
        }
        return Ok(());
    }

    report_drift(&drift);
    eprintln!(
        "protocol-drift: if this change is intentional, run `cargo xtask check-protocol-drift --update`"
    );

    if hook || warn_only {
        return Ok(());
    }

    bail!("core contract drift detected");
}

fn calculate_lockfile(root: &Path) -> Result<Lockfile> {
    let mut surfaces = Vec::with_capacity(SURFACES.len());

    for surface in SURFACES {
        let path = root.join(surface.path);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let normalized = normalize_contract_source(&content);
        surfaces.push(SurfaceHash {
            name: surface.name.to_string(),
            path: surface.path.to_string(),
            hash: hash_contract(&normalized),
        });
    }

    Ok(Lockfile {
        version: 1,
        algorithm: ALGORITHM.to_string(),
        surfaces,
    })
}

fn read_lockfile(path: &Path) -> Result<Lockfile> {
    let content = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read {}; run `cargo xtask check-protocol-drift --update` to create it",
            path.display()
        )
    })?;
    let lockfile: Lockfile = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if lockfile.version != 1 {
        bail!(
            "unsupported protocol drift lockfile version {} in {}",
            lockfile.version,
            path.display()
        );
    }
    if lockfile.algorithm != ALGORITHM {
        bail!(
            "unsupported protocol drift algorithm {} in {} (expected {ALGORITHM})",
            lockfile.algorithm,
            path.display()
        );
    }

    Ok(lockfile)
}

fn write_lockfile(path: &Path, lockfile: &Lockfile) -> Result<()> {
    let mut content = serde_json::to_string_pretty(lockfile)?;
    content.push('\n');
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn compare_lockfiles(expected: &Lockfile, actual: &Lockfile) -> Vec<Drift> {
    let mut drift = Vec::new();

    for expected_surface in &expected.surfaces {
        let actual_surface = actual
            .surfaces
            .iter()
            .find(|surface| surface.name == expected_surface.name);

        match actual_surface {
            Some(actual_surface) if actual_surface.hash == expected_surface.hash => {}
            Some(actual_surface) => drift.push(Drift {
                name: expected_surface.name.clone(),
                path: expected_surface.path.clone(),
                expected: Some(expected_surface.hash.clone()),
                actual: Some(actual_surface.hash.clone()),
            }),
            None => drift.push(Drift {
                name: expected_surface.name.clone(),
                path: expected_surface.path.clone(),
                expected: Some(expected_surface.hash.clone()),
                actual: None,
            }),
        }
    }

    for actual_surface in &actual.surfaces {
        if expected
            .surfaces
            .iter()
            .all(|surface| surface.name != actual_surface.name)
        {
            drift.push(Drift {
                name: actual_surface.name.clone(),
                path: actual_surface.path.clone(),
                expected: None,
                actual: Some(actual_surface.hash.clone()),
            });
        }
    }

    drift
}

fn report_drift(drift: &[Drift]) {
    eprintln!("protocol-drift: core contract hash mismatch:");
    for item in drift {
        eprintln!("  - {} ({})", item.name, item.path);
        match (&item.expected, &item.actual) {
            (Some(expected), Some(actual)) => {
                eprintln!("      expected: {expected}");
                eprintln!("      actual  : {actual}");
            }
            (Some(expected), None) => {
                eprintln!("      expected: {expected}");
                eprintln!("      actual  : <missing>");
            }
            (None, Some(actual)) => {
                eprintln!("      expected: <missing>");
                eprintln!("      actual  : {actual}");
            }
            (None, None) => {}
        }
    }
}

fn normalize_contract_source(content: &str) -> String {
    let mut normalized = Vec::new();
    let mut skip_cfg_test = false;
    let mut skipping_test_module = false;
    let mut test_module_depth = 0isize;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();

        if skipping_test_module {
            test_module_depth += brace_delta(trimmed);
            if test_module_depth <= 0 {
                skipping_test_module = false;
                test_module_depth = 0;
            }
            continue;
        }

        if trimmed == "#[cfg(test)]" {
            skip_cfg_test = true;
            continue;
        }

        if skip_cfg_test && trimmed.starts_with("mod tests") {
            test_module_depth = brace_delta(trimmed);
            if test_module_depth > 0 {
                skipping_test_module = true;
            }
            skip_cfg_test = false;
            continue;
        }

        if skip_cfg_test {
            skip_cfg_test = false;
        }

        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        let line = strip_trailing_comment(trimmed).trim();
        if !line.is_empty() {
            normalized.push(line.to_string());
        }
    }

    normalized.join("\n") + "\n"
}

fn strip_trailing_comment(line: &str) -> &str {
    // NOTE: splits on " //" which may incorrectly trim content inside string literals
    // (e.g. `let url = "https://foo.com";`). The space-prefix guard handles most cases;
    // the surfaces tracked by this tool don't exhibit the problematic pattern.
    line.split_once(" //")
        .map_or(line, |(before_comment, _)| before_comment)
}

fn brace_delta(line: &str) -> isize {
    // NOTE: counts raw brace characters without accounting for braces inside string or
    // character literals (e.g. `let s = "{";` counts as +1). This can misfire on
    // edge-case source files, but the surfaces tracked by this tool don't exhibit it.
    line.chars().fold(0, |delta, ch| match ch {
        '{' => delta + 1,
        '}' => delta - 1,
        _ => delta,
    })
}

fn hash_contract(normalized: &str) -> String {
    let digest = Sha256::digest(normalized.as_bytes());
    hex::encode(digest)
}

fn read_hook_file_path() -> Result<Option<PathBuf>> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read hook input from stdin")?;

    if input.trim().is_empty() {
        return Ok(None);
    }

    let hook_input: HookInput =
        serde_json::from_str(&input).context("failed to parse hook input JSON")?;
    Ok(hook_input
        .tool_input
        .and_then(|tool_input| tool_input.file_path))
}

fn is_tracked_surface_path(root: &Path, path: &Path) -> bool {
    let relative = display_relative(root, path);
    let relative = relative.to_string_lossy();
    SURFACES.iter().any(|surface| relative == surface.path)
}

fn display_relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_hash_ignores_comments_and_tests() {
        let first = normalize_contract_source(
            r#"
            //! docs
            #[derive(Debug)]
            pub enum Wire {
                Request,
            }

            #[cfg(test)]
            mod tests {
                #[test]
                fn ignored() {
                    assert_eq!(1, 1);
                }
            }
            "#,
        );

        let second = normalize_contract_source(
            r#"
            // different docs should not affect the contract hash
            #[derive(Debug)]
            pub enum Wire {
                Request,
            }

            #[cfg(test)]
            mod tests {
                #[test]
                fn also_ignored() {
                    assert_eq!(2, 2);
                }
            }
            "#,
        );

        assert_eq!(hash_contract(&first), hash_contract(&second));
    }

    #[test]
    fn contract_hash_changes_when_contract_changes() {
        let before = normalize_contract_source("pub enum Wire { Request }\n");
        let after = normalize_contract_source("pub enum Wire { Request, Response }\n");

        assert_ne!(hash_contract(&before), hash_contract(&after));
    }

    #[test]
    fn all_expected_surfaces_are_tracked() {
        let expected = [
            "wire-protocol",
            "domain-ports",
            "domain-networking",
            "domain-extensions",
            "lifecycle-events",
            "execution-manifest",
            "execution-policy",
            "error-types",
            "client-api",
            "typestate",
        ];
        let tracked: Vec<&str> = SURFACES.iter().map(|s| s.name).collect();
        for name in &expected {
            assert!(
                tracked.contains(name),
                "expected surface '{name}' is not tracked"
            );
        }
        assert_eq!(
            tracked.len(),
            expected.len(),
            "surface count mismatch: tracked {tracked:?} vs expected {expected:?}"
        );
    }

    #[test]
    fn identifies_tracked_surface_paths() {
        let root = Path::new("/repo");

        assert!(is_tracked_surface_path(
            root,
            Path::new("/repo/crates/minibox-core/src/protocol.rs")
        ));
        assert!(is_tracked_surface_path(
            root,
            Path::new("crates/minibox-core/src/domain/networking.rs")
        ));
        assert!(is_tracked_surface_path(
            root,
            Path::new("/repo/crates/minibox-core/src/domain/execution_manifest.rs")
        ));
        assert!(is_tracked_surface_path(
            root,
            Path::new("/repo/crates/minibox-core/src/typestate.rs")
        ));
        assert!(!is_tracked_surface_path(
            root,
            Path::new("/repo/crates/minibox/src/daemon/handler.rs")
        ));
    }

    #[test]
    fn compare_lockfiles_reports_hash_mismatch() {
        let expected = Lockfile {
            version: 1,
            algorithm: ALGORITHM.to_string(),
            surfaces: vec![SurfaceHash {
                name: "wire-protocol".to_string(),
                path: "crates/minibox-core/src/protocol.rs".to_string(),
                hash: "old".to_string(),
            }],
        };
        let actual = Lockfile {
            version: 1,
            algorithm: ALGORITHM.to_string(),
            surfaces: vec![SurfaceHash {
                name: "wire-protocol".to_string(),
                path: "crates/minibox-core/src/protocol.rs".to_string(),
                hash: "new".to_string(),
            }],
        };

        assert_eq!(
            compare_lockfiles(&expected, &actual),
            vec![Drift {
                name: "wire-protocol".to_string(),
                path: "crates/minibox-core/src/protocol.rs".to_string(),
                expected: Some("old".to_string()),
                actual: Some("new".to_string()),
            }]
        );
    }
}
