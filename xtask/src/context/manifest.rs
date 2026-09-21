//! Collects manifest data for the xtask context snapshot.

use super::model::{AdapterMaturity, CapabilitySupport};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::parse::Parser;
use syn::visit::Visit;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct ContextManifest {
    pub(super) schema_version: u32,
    pub(super) adapters: Vec<AdapterDeclaration>,
    pub(super) profiles: Vec<ValidationProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct AdapterDeclaration {
    pub(super) id: String,
    pub(super) maturity: AdapterMaturity,
    pub(super) platforms: Vec<String>,
    pub(super) default_roles: Vec<String>,
    pub(super) capabilities: BTreeMap<String, CapabilitySupport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(super) struct ValidationProfile {
    pub(super) id: String,
    pub(super) target: String,
    pub(super) features: Vec<String>,
    pub(super) no_default_features: bool,
    pub(super) all_targets: bool,
    pub(super) required_in_ci: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AdapterRegistryObservation {
    pub(super) adapter_ids: BTreeSet<String>,
    pub(super) platforms: BTreeMap<String, String>,
    pub(super) default_roles: BTreeMap<String, String>,
}

pub(super) fn parse_adapter_registry(source: &str) -> Result<AdapterRegistryObservation> {
    let syntax = syn::parse_file(source).context("parse adapter_registry.rs")?;
    let mut suite_variants = BTreeSet::new();
    let mut suite_ids = BTreeMap::new();
    let mut valid_ids = BTreeSet::new();
    let mut default_adapter = None;
    let mut fallback_adapters = Vec::new();
    let mut adapters = AdapterInfoVisitor::default();

    for item in &syntax.items {
        match item {
            syn::Item::Enum(item) if item.ident == "AdapterSuite" => {
                suite_variants.extend(
                    item.variants
                        .iter()
                        .map(|variant| variant.ident.to_string()),
                );
            }
            syn::Item::Impl(item) if impl_type_name(item).as_deref() == Some("AdapterSuite") => {
                for impl_item in &item.items {
                    if let syn::ImplItem::Fn(method) = impl_item
                        && method.sig.ident == "as_str"
                    {
                        let mut visitor = AdapterSuiteStringVisitor::default();
                        visitor.visit_block(&method.block);
                        suite_ids.extend(visitor.ids);
                    }
                }
            }
            syn::Item::Const(item) if item.ident == "VALID_ADAPTERS" => {
                valid_ids.extend(string_literals(&item.expr));
            }
            syn::Item::Const(item) if item.ident == "DEFAULT_ADAPTER_SUITE" => {
                default_adapter = string_literals(&item.expr).into_iter().next();
            }
            syn::Item::Const(item) if item.ident == "FALLBACK_ADAPTER_SUITE" => {
                fallback_adapters = fallback_adapter_ids(&item.expr)?;
            }
            syn::Item::Fn(item) if item.sig.ident == "all_adapters" => {
                adapters.visit_item_fn(item);
            }
            _ => {}
        }
    }

    let mapped_variants = suite_variants
        .iter()
        .map(|variant| {
            suite_ids
                .get(variant)
                .cloned()
                .with_context(|| format!("AdapterSuite::{variant} is missing from as_str"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let observed_ids = adapters.platforms.keys().cloned().collect::<BTreeSet<_>>();
    if mapped_variants != valid_ids || valid_ids != observed_ids {
        bail!(
            "adapter registry declarations disagree: suite={mapped_variants:?}, valid={valid_ids:?}, all={observed_ids:?}"
        );
    }
    let default_adapter = default_adapter.context("DEFAULT_ADAPTER_SUITE is missing")?;
    if fallback_adapters.len() != 2 {
        bail!("FALLBACK_ADAPTER_SUITE must declare Linux and non-Linux adapters");
    }
    Ok(AdapterRegistryObservation {
        adapter_ids: observed_ids,
        platforms: adapters.platforms,
        default_roles: BTreeMap::from([
            ("unix_default".to_string(), default_adapter),
            ("linux_fallback".to_string(), fallback_adapters[0].clone()),
            ("macos_fallback".to_string(), fallback_adapters[1].clone()),
        ]),
    })
}

fn impl_type_name(item: &syn::ItemImpl) -> Option<String> {
    let syn::Type::Path(path) = item.self_ty.as_ref() else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

#[derive(Default)]
struct AdapterSuiteStringVisitor {
    ids: BTreeMap<String, String>,
}

impl<'ast> Visit<'ast> for AdapterSuiteStringVisitor {
    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        for arm in &expression.arms {
            let syn::Pat::Path(path) = &arm.pat else {
                continue;
            };
            let Some(variant) = path.path.segments.last() else {
                continue;
            };
            if let Some(id) = expression_string(&arm.body) {
                self.ids.insert(variant.ident.to_string(), id);
            }
        }
        syn::visit::visit_expr_match(self, expression);
    }
}

#[derive(Default)]
struct AdapterInfoVisitor {
    platforms: BTreeMap<String, String>,
}

impl<'ast> Visit<'ast> for AdapterInfoVisitor {
    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if expression
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "vec")
            && let Ok(items) =
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(expression.mac.tokens.clone())
        {
            for item in &items {
                self.visit_expr(item);
            }
        }
        syn::visit::visit_expr_macro(self, expression);
    }

    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "AdapterInfo")
        {
            let mut name = None;
            let mut platform = None;
            for field in &expression.fields {
                let syn::Member::Named(member) = &field.member else {
                    continue;
                };
                match member.to_string().as_str() {
                    "name" => name = expression_string(&field.expr),
                    "platform" => platform = expression_string(&field.expr),
                    _ => {}
                }
            }
            if let (Some(name), Some(platform)) = (name, platform) {
                self.platforms.insert(name, platform);
            }
        }
        syn::visit::visit_expr_struct(self, expression);
    }
}

fn fallback_adapter_ids(expression: &syn::Expr) -> Result<Vec<String>> {
    let syn::Expr::If(expression) = expression else {
        bail!("FALLBACK_ADAPTER_SUITE must be an if expression");
    };
    let linux = block_string(&expression.then_branch)
        .context("FALLBACK_ADAPTER_SUITE Linux branch is missing an adapter")?;
    let (_, alternative) = expression
        .else_branch
        .as_ref()
        .context("FALLBACK_ADAPTER_SUITE is missing a non-Linux branch")?;
    let other = match alternative.as_ref() {
        syn::Expr::Block(block) => block_string(&block.block),
        expression => expression_string(expression),
    }
    .context("FALLBACK_ADAPTER_SUITE non-Linux branch is missing an adapter")?;
    Ok(vec![linux, other])
}

fn block_string(block: &syn::Block) -> Option<String> {
    let syn::Stmt::Expr(expression, _) = block.stmts.last()? else {
        return None;
    };
    expression_string(expression)
}

fn expression_string(expression: &syn::Expr) -> Option<String> {
    let syn::Expr::Lit(literal) = expression else {
        return None;
    };
    let syn::Lit::Str(value) = &literal.lit else {
        return None;
    };
    Some(value.value())
}

fn string_literals(expression: &syn::Expr) -> Vec<String> {
    #[derive(Default)]
    struct StringVisitor(Vec<String>);

    impl<'ast> Visit<'ast> for StringVisitor {
        fn visit_lit_str(&mut self, value: &'ast syn::LitStr) {
            self.0.push(value.value());
        }
    }

    let mut visitor = StringVisitor::default();
    visitor.visit_expr(expression);
    visitor.0
}

pub(super) fn load_manifest(root: &Path) -> Result<ContextManifest> {
    let path = root.join("xtask/context.toml");
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("reading xtask/context.toml at {}", path.display()))?;
    toml::from_str(&content)
        .with_context(|| format!("parsing xtask/context.toml at {}", path.display()))
}

pub(super) fn validate_manifest(manifest: &ContextManifest) -> Result<()> {
    let mut diagnostics = Vec::new();

    if manifest.schema_version != 1 {
        diagnostics.push(format!(
            "unsupported context manifest schema version {}",
            manifest.schema_version
        ));
    }
    for adapter in &manifest.adapters {
        if adapter.id.is_empty() {
            diagnostics.push("empty adapter id".to_string());
        }
        if adapter.platforms.is_empty() {
            diagnostics.push(format!("adapter {} has no platforms", adapter.id));
        }
        for role in &adapter.default_roles {
            if !matches!(
                role.as_str(),
                "unix_default" | "linux_fallback" | "macos_fallback"
            ) {
                diagnostics.push(format!("unknown default role: {role}"));
            }
        }
    }
    for profile in &manifest.profiles {
        if profile.id.is_empty() {
            diagnostics.push("empty profile id".to_string());
        }
        if profile.target.is_empty() {
            diagnostics.push(format!("profile {} has no target", profile.id));
        }
    }

    collect_duplicate_ids(
        manifest.adapters.iter().map(|adapter| adapter.id.as_str()),
        "adapter",
        &mut diagnostics,
    );
    collect_duplicate_ids(
        manifest.profiles.iter().map(|profile| profile.id.as_str()),
        "profile",
        &mut diagnostics,
    );

    let mut profile_keys = BTreeSet::new();
    for profile in &manifest.profiles {
        let mut features = profile.features.clone();
        features.sort();
        let key = (profile.target.clone(), features);
        if !profile_keys.insert(key) {
            diagnostics.push(format!(
                "duplicate target/feature profile: {}",
                profile.target
            ));
        }
    }

    validate_capability_keys(&manifest.adapters, &mut diagnostics);

    for target in [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc",
    ] {
        let has_native_profile = manifest.profiles.iter().any(|profile| {
            profile.target == target && profile.features.is_empty() && !profile.no_default_features
        });
        if !has_native_profile {
            diagnostics.push(format!("missing native profile: {target}"));
        }
    }

    let adapter_ids: BTreeSet<&str> = manifest
        .adapters
        .iter()
        .map(|adapter| adapter.id.as_str())
        .collect();
    for adapter in &manifest.adapters {
        for role in &adapter.default_roles {
            let expected_adapter = match role.as_str() {
                "unix_default" => Some("smolvm"),
                "linux_fallback" => Some("native"),
                "macos_fallback" => Some("krun"),
                _ => None,
            };
            if let Some(expected_adapter) = expected_adapter {
                if !adapter_ids.contains(expected_adapter) {
                    diagnostics.push(format!(
                        "default role {role} references undeclared adapter: {expected_adapter}"
                    ));
                } else if adapter.id != expected_adapter {
                    diagnostics.push(format!(
                        "default role {role} must reference adapter: {expected_adapter}"
                    ));
                }
            }
        }
    }

    diagnostics.sort();
    diagnostics.dedup();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(diagnostics.join("\n"))
    }
}

fn collect_duplicate_ids<'a>(
    ids: impl Iterator<Item = &'a str>,
    kind: &str,
    diagnostics: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            diagnostics.push(format!("duplicate {kind} id: {id}"));
        }
    }
}

fn validate_capability_keys(adapters: &[AdapterDeclaration], diagnostics: &mut Vec<String>) {
    let mut frequencies = BTreeMap::<Vec<&str>, usize>::new();
    for adapter in adapters {
        let keys = adapter
            .capabilities
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        *frequencies.entry(keys).or_default() += 1;
    }
    let canonical = frequencies
        .into_iter()
        .max_by(|(left_keys, left_count), (right_keys, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_keys.cmp(left_keys))
        })
        .map(|(keys, _)| keys);

    if let Some(canonical) = canonical {
        for adapter in adapters {
            let keys = adapter
                .capabilities
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if keys != canonical {
                diagnostics.push(format!(
                    "capability keys differ for adapter: {}",
                    adapter.id
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::collect::reconcile_adapters;
    use crate::context::model::ValidationState;

    const VALID_MANIFEST: &str = r#"
schema_version = 1

[[adapters]]
id = "native"
maturity = "production"
platforms = ["linux"]
default_roles = []

[adapters.capabilities]
run = "yes"

[[profiles]]
id = "native-linux"
target = "x86_64-unknown-linux-gnu"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
"#;

    #[test]
    fn manifest_loader_reports_path_and_parse_errors() {
        let valid_root = tempfile::tempdir().expect("valid fixture root should be created");
        std::fs::create_dir(valid_root.path().join("xtask"))
            .expect("fixture xtask directory should be created");
        std::fs::write(valid_root.path().join("xtask/context.toml"), VALID_MANIFEST)
            .expect("valid manifest fixture should be written");

        let manifest = load_manifest(valid_root.path()).expect("valid manifest should load");
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.adapters.len(), 1);
        assert_eq!(manifest.adapters[0].id, "native");
        assert_eq!(manifest.adapters[0].maturity, AdapterMaturity::Production);
        assert_eq!(
            manifest.adapters[0].capabilities["run"],
            CapabilitySupport::Yes
        );
        assert_eq!(manifest.profiles.len(), 1);
        assert_eq!(manifest.profiles[0].id, "native-linux");
        assert!(manifest.profiles[0].all_targets);

        let missing_root = tempfile::tempdir().expect("missing fixture root should be created");
        let missing_error = load_manifest(missing_root.path())
            .expect_err("missing manifest should return an error")
            .to_string();
        assert!(
            missing_error.contains("xtask/context.toml"),
            "{missing_error}"
        );

        let invalid_root = tempfile::tempdir().expect("invalid fixture root should be created");
        std::fs::create_dir(invalid_root.path().join("xtask"))
            .expect("fixture xtask directory should be created");
        std::fs::write(
            invalid_root.path().join("xtask/context.toml"),
            r#"schema_version = "invalid""#,
        )
        .expect("invalid manifest fixture should be written");
        let invalid_error = load_manifest(invalid_root.path())
            .expect_err("invalid manifest should return an error")
            .to_string();
        assert!(
            invalid_error.contains("xtask/context.toml"),
            "{invalid_error}"
        );
    }
    fn validation_fixture() -> ContextManifest {
        let capabilities = BTreeMap::from([
            ("pull".to_string(), CapabilitySupport::Yes),
            ("run".to_string(), CapabilitySupport::Yes),
        ]);
        let adapter = |id: &str| AdapterDeclaration {
            id: id.to_string(),
            maturity: AdapterMaturity::Production,
            platforms: vec!["any".to_string()],
            default_roles: Vec::new(),
            capabilities: capabilities.clone(),
        };
        let profile = |id: &str, target: &str| ValidationProfile {
            id: id.to_string(),
            target: target.to_string(),
            features: Vec::new(),
            no_default_features: false,
            all_targets: true,
            required_in_ci: true,
        };

        ContextManifest {
            schema_version: 1,
            adapters: vec![adapter("krun"), adapter("native"), adapter("smolvm")],
            profiles: vec![
                profile("native-macos", "aarch64-apple-darwin"),
                profile("native-linux-gnu", "x86_64-unknown-linux-gnu"),
                profile("native-linux-musl", "x86_64-unknown-linux-musl"),
                profile("native-windows", "x86_64-pc-windows-msvc"),
            ],
        }
    }

    fn assert_manifest_invalid(manifest: &ContextManifest, expected: &str) {
        let error = validate_manifest(manifest)
            .expect_err("ambiguous manifest should be rejected")
            .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
    }

    #[test]
    fn manifest_rejects_ambiguous_declarations() {
        validate_manifest(&validation_fixture()).expect("empty default roles should remain valid");

        let mut duplicate_adapter = validation_fixture();
        duplicate_adapter
            .adapters
            .push(duplicate_adapter.adapters[0].clone());
        assert_manifest_invalid(&duplicate_adapter, "duplicate adapter id: krun");

        let mut duplicate_profile_id = validation_fixture();
        let mut duplicate = duplicate_profile_id.profiles[0].clone();
        duplicate.target = "aarch64-unknown-linux-gnu".to_string();
        duplicate_profile_id.profiles.push(duplicate);
        assert_manifest_invalid(&duplicate_profile_id, "duplicate profile id: native-macos");

        let mut duplicate_profile_key = validation_fixture();
        let mut duplicate = duplicate_profile_key.profiles[0].clone();
        duplicate.id = "same-target-and-features".to_string();
        duplicate_profile_key.profiles.push(duplicate);
        assert_manifest_invalid(
            &duplicate_profile_key,
            "duplicate target/feature profile: aarch64-apple-darwin",
        );

        let mut unequal_capabilities = validation_fixture();
        unequal_capabilities.adapters[0].capabilities.remove("pull");
        assert_manifest_invalid(
            &unequal_capabilities,
            "capability keys differ for adapter: krun",
        );

        let mut missing_native_profile = validation_fixture();
        missing_native_profile
            .profiles
            .retain(|profile| profile.target != "x86_64-pc-windows-msvc");
        assert_manifest_invalid(
            &missing_native_profile,
            "missing native profile: x86_64-pc-windows-msvc",
        );

        let mut unresolved_default_role = validation_fixture();
        unresolved_default_role
            .adapters
            .retain(|adapter| adapter.id != "smolvm");
        unresolved_default_role.adapters[0].default_roles = vec!["unix_default".to_string()];
        assert_manifest_invalid(
            &unresolved_default_role,
            "default role unix_default references undeclared adapter: smolvm",
        );
    }
    #[test]
    fn repository_manifest_declares_exact_adapters_and_profiles() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should have a workspace root");
        let manifest = load_manifest(root).expect("repository context manifest should load");
        validate_manifest(&manifest).expect("repository context manifest should be valid");

        let mut adapter_ids = manifest
            .adapters
            .iter()
            .map(|adapter| adapter.id.as_str())
            .collect::<Vec<_>>();
        adapter_ids.sort_unstable();
        assert_eq!(
            adapter_ids,
            ["colima", "gke", "krun", "native", "smolvm", "vz", "winbox"]
        );

        let mut profile_ids = manifest
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>();
        profile_ids.sort_unstable();
        assert_eq!(
            profile_ids,
            [
                "macos-vz",
                "native-linux-gnu",
                "native-linux-musl",
                "native-macos",
                "native-windows",
            ]
        );

        let vz = manifest
            .adapters
            .iter()
            .find(|adapter| adapter.id == "vz")
            .expect("VZ declaration should exist");
        assert_eq!(vz.maturity, AdapterMaturity::Blocked);
        assert!(
            vz.capabilities.values().all(|support| matches!(
                support,
                CapabilitySupport::Blocked | CapabilitySupport::No
            ))
        );

        let default_roles = manifest
            .adapters
            .iter()
            .flat_map(|adapter| {
                adapter
                    .default_roles
                    .iter()
                    .map(move |role| (role.as_str(), adapter.id.as_str()))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(default_roles.get("unix_default"), Some(&"smolvm"));
        assert_eq!(default_roles.get("linux_fallback"), Some(&"native"));
        assert_eq!(default_roles.get("macos_fallback"), Some(&"krun"));
    }
    #[test]
    fn adapter_reconciliation_reports_registry_drift() {
        let source = r#"
pub enum AdapterSuite { Native, Krun }
impl AdapterSuite {
    pub const fn as_str(&self) -> &str {
        match self { Self::Native => "native", Self::Krun => "krun" }
    }
}

pub const VALID_ADAPTERS: &[&str] = &["native", "krun"];
pub const DEFAULT_ADAPTER_SUITE: &str = "native";
pub const FALLBACK_ADAPTER_SUITE: &str = if cfg!(target_os = "linux") { "native" } else { "krun" };
pub fn all_adapters() -> Vec<AdapterInfo> {
    vec![
        AdapterInfo { name: "native", available: true, platform: "linux" },
        AdapterInfo { name: "krun", available: true, platform: "macos" },
    ]
}
"#;
        let registry = parse_adapter_registry(source).expect("registry fixture should parse");
        assert_eq!(
            registry
                .adapter_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["krun", "native"]
        );

        let adapter = |id: &str, platform: &str, roles: &[&str]| AdapterDeclaration {
            id: id.to_string(),
            maturity: AdapterMaturity::Production,
            platforms: vec![platform.to_string()],
            default_roles: roles.iter().map(ToString::to_string).collect(),
            capabilities: BTreeMap::from([("run".to_string(), CapabilitySupport::Yes)]),
        };
        let manifest = ContextManifest {
            schema_version: 1,
            adapters: vec![
                adapter("native", "linux", &["unix_default", "linux_fallback"]),
                adapter("krun", "macos", &["macos_fallback"]),
            ],
            profiles: Vec::new(),
        };
        let tested_capabilities = BTreeMap::from([
            (
                "native".to_string(),
                BTreeMap::from([("run".to_string(), CapabilitySupport::Yes)]),
            ),
            (
                "krun".to_string(),
                BTreeMap::from([("run".to_string(), CapabilitySupport::Yes)]),
            ),
        ]);
        let exact = reconcile_adapters(&manifest, &registry, &tested_capabilities);
        assert!(exact.diagnostics.is_empty());
        assert!(exact.adapters.iter().all(|adapter| {
            adapter.maturity.observed.is_none()
                && adapter.maturity.validation.state == ValidationState::DeclaredOnly
        }));

        let mut missing = registry.clone();
        missing.adapter_ids.remove("krun");
        missing.platforms.remove("krun");
        let missing = reconcile_adapters(&manifest, &missing, &tested_capabilities);
        assert!(
            missing
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.registry_missing")
        );

        let mut stub_manifest = manifest.clone();
        stub_manifest.adapters.push(AdapterDeclaration {
            id: "winbox".to_string(),
            maturity: AdapterMaturity::Stub,
            platforms: vec!["windows".to_string()],
            default_roles: Vec::new(),
            capabilities: BTreeMap::from([("run".to_string(), CapabilitySupport::No)]),
        });
        let stub = reconcile_adapters(&stub_manifest, &registry, &tested_capabilities);
        assert!(!stub.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "adapter.registry_missing" && diagnostic.message.contains("winbox")
        }));

        let mut extra = registry.clone();
        extra.adapter_ids.insert("extra".to_string());
        extra
            .platforms
            .insert("extra".to_string(), "linux".to_string());
        let extra = reconcile_adapters(&manifest, &extra, &tested_capabilities);
        assert!(
            extra
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.registry_extra")
        );

        let mut platform = registry.clone();
        platform
            .platforms
            .insert("native".to_string(), "macos".to_string());
        let platform = reconcile_adapters(&manifest, &platform, &tested_capabilities);
        assert!(
            platform
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.platform_mismatch")
        );

        let mut defaults = registry.clone();
        defaults
            .default_roles
            .insert("unix_default".to_string(), "krun".to_string());
        let defaults = reconcile_adapters(&manifest, &defaults, &tested_capabilities);
        assert!(
            defaults
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.default_mismatch")
        );

        let unsupported = reconcile_adapters(&manifest, &registry, &BTreeMap::new());
        assert!(
            unsupported
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.capability_unvalidated")
        );
        assert!(unsupported.adapters.iter().all(|adapter| {
            adapter.capabilities["run"].observed.is_none()
                && adapter.capabilities["run"].validation.state == ValidationState::DeclaredOnly
        }));

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask should have a workspace root");
        let live_source =
            std::fs::read_to_string(root.join("crates/miniboxd/src/adapter_registry.rs"))
                .expect("live adapter registry should be readable");
        let live_registry =
            parse_adapter_registry(&live_source).expect("live adapter registry should parse");
        let live_manifest = load_manifest(root).expect("live manifest should load");
        let live = reconcile_adapters(&live_manifest, &live_registry, &BTreeMap::new());
        let vz = live
            .adapters
            .iter()
            .find(|adapter| adapter.id == "vz")
            .expect("VZ should remain declared");
        assert_eq!(vz.maturity.declared, Some(AdapterMaturity::Blocked));
        assert_eq!(vz.registry_presence.observed, Some(true));
        assert!(
            vz.capabilities
                .values()
                .all(|capability| capability.observed.is_none())
        );
    }

    #[test]
    fn adapter_reconciliation_normalizes_runtime_any_platform() {
        let mut manifest = validation_fixture();
        manifest.adapters = vec![AdapterDeclaration {
            id: "portable".to_string(),
            maturity: AdapterMaturity::Production,
            platforms: vec!["linux".to_string(), "macos".to_string()],
            default_roles: Vec::new(),
            capabilities: BTreeMap::new(),
        }];
        let registry = AdapterRegistryObservation {
            adapter_ids: BTreeSet::from(["portable".to_string()]),
            platforms: BTreeMap::from([("portable".to_string(), "any".to_string())]),
            default_roles: BTreeMap::new(),
        };
        let result = reconcile_adapters(&manifest, &registry, &BTreeMap::new());
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "adapter.platform_mismatch")
        );
        assert_eq!(
            result.adapters[0].platforms.observed,
            Some(vec!["linux".to_string(), "macos".to_string()])
        );
    }

    #[test]
    fn adapter_registry_parser_rejects_incomplete_declarations() {
        let valid = r#"
pub enum AdapterSuite { Native, Krun }
impl AdapterSuite { pub const fn as_str(&self) -> &str { match self { Self::Native => "native", Self::Krun => "krun" } } }
pub const VALID_ADAPTERS: &[&str] = &["native", "krun"];
pub const DEFAULT_ADAPTER_SUITE: &str = "native";
pub const FALLBACK_ADAPTER_SUITE: &str = if cfg!(target_os = "linux") { "native" } else { "krun" };
pub fn all_adapters() -> Vec<AdapterInfo> { vec![AdapterInfo { name: "native", platform: "linux" }, AdapterInfo { name: "krun", platform: "macos" }] }
"#;
        for (needle, replacement) in [
            ("Self::Krun => \"krun\"", "Self::Krun => value"),
            ("pub const DEFAULT_ADAPTER_SUITE: &str = \"native\";", ""),
            (" else { \"krun\" }", ""),
            ("[\"native\", \"krun\"]", "[\"native\"]"),
            (
                "name: \"krun\", platform: \"macos\"",
                "name: \"other\", platform: \"macos\"",
            ),
        ] {
            assert!(parse_adapter_registry(&valid.replacen(needle, replacement, 1)).is_err());
        }
        assert!(parse_adapter_registry("not rust }").is_err());
    }

    #[test]
    fn manifest_validation_reports_docs_audit_errors() {
        let mut manifest = validation_fixture();
        manifest.schema_version = 2;
        manifest.adapters[0].id.clear();
        manifest.adapters[0].platforms.clear();
        manifest.adapters[0].default_roles = vec!["unknown".to_string()];
        manifest.profiles[0].target.clear();
        let error = validate_manifest(&manifest)
            .expect_err("invalid manifest should fail")
            .to_string();
        for expected in [
            "unsupported context manifest schema version",
            "empty adapter id",
            "has no platforms",
            "unknown default role",
            "has no target",
        ] {
            assert!(
                error.contains(expected),
                "missing {expected:?} in {error:?}"
            );
        }
    }
}
