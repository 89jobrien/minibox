use super::collect::RepositoryReader;
use super::model::{RepositoryPath, SourceTestDeclaration, TestDeclarationKind, WorkspaceSnapshot};
use anyhow::{Context, Result, bail};
use std::path::Path;
use syn::visit::Visit;

pub(super) fn collect_source_tests(
    repository: &impl RepositoryReader,
    root: &Path,
    workspace: &WorkspaceSnapshot,
) -> Result<Vec<SourceTestDeclaration>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", root.display()))?;
    let tracked_files = repository.tracked_files(&root)?;
    let mut declarations = Vec::new();

    for path in tracked_files {
        if Path::new(path.as_str())
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("rs")
        {
            continue;
        }
        let Some(package_index) = owning_package_index(&path, workspace) else {
            continue;
        };
        let package = &workspace.packages[package_index];
        let source = repository.read_utf8(&root.join(path.as_str()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parse Rust source {}", path.as_str()))?;
        let mut visitor = DeclarationVisitor {
            package_id: package.package_id.clone(),
            path,
            module_path: Vec::new(),
            inherited_cfg: Vec::new(),
            declarations: Vec::new(),
            error: None,
        };
        visitor.visit_file(&syntax);
        if let Some(error) = visitor.error {
            return Err(error);
        }
        declarations.extend(visitor.declarations);
    }

    declarations.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    Ok(declarations)
}

fn owning_package_index(path: &RepositoryPath, workspace: &WorkspaceSnapshot) -> Option<usize> {
    let path = Path::new(path.as_str());
    workspace
        .packages
        .iter()
        .enumerate()
        .filter_map(|(index, package)| {
            let root = Path::new(package.manifest_path.as_str()).parent()?;
            path.starts_with(root)
                .then_some((index, root.components().count()))
        })
        .max_by_key(|(_, depth)| *depth)
        .map(|(index, _)| index)
}

struct DeclarationVisitor {
    package_id: String,
    path: RepositoryPath,
    module_path: Vec<String>,
    inherited_cfg: Vec<String>,
    declarations: Vec<SourceTestDeclaration>,
    error: Option<anyhow::Error>,
}

impl DeclarationVisitor {
    fn record(&mut self, name: String, kind: TestDeclarationKind, attributes: &[syn::Attribute]) {
        let mut cfg_predicates = self.inherited_cfg.clone();
        cfg_predicates.extend(cfg_predicates_from(attributes));
        cfg_predicates.sort();
        cfg_predicates.dedup();
        let module = if self.module_path.is_empty() {
            "<root>".to_string()
        } else {
            self.module_path.join("::")
        };
        let kind_name = match kind {
            TestDeclarationKind::Function => "function",
            TestDeclarationKind::MacroInvocation => "macro_invocation",
        };
        self.declarations.push(SourceTestDeclaration {
            stable_id: format!(
                "{}::{}::{module}::{name}::{kind_name}",
                self.package_id,
                self.path.as_str()
            ),
            package_id: self.package_id.clone(),
            path: self.path.clone(),
            module_path: self.module_path.clone(),
            name,
            cfg_predicates,
            kind,
        });
    }
}

impl<'ast> Visit<'ast> for DeclarationVisitor {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let module_depth = self.module_path.len();
        let cfg_depth = self.inherited_cfg.len();
        self.module_path.push(item.ident.to_string());
        self.inherited_cfg.extend(cfg_predicates_from(&item.attrs));
        if let Some((_, items)) = &item.content {
            for item in items {
                self.visit_item(item);
            }
        }
        self.module_path.truncate(module_depth);
        self.inherited_cfg.truncate(cfg_depth);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_test_attribute) {
            self.record(
                item.sig.ident.to_string(),
                TestDeclarationKind::Function,
                &item.attrs,
            );
        }
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "conformance_test")
        {
            match conformance_test_name(item) {
                Ok(name) => self.record(name, TestDeclarationKind::MacroInvocation, &item.attrs),
                Err(error) => self.error = Some(error),
            }
        }
    }
}

fn is_test_attribute(attribute: &syn::Attribute) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
}

fn cfg_predicates_from(attributes: &[syn::Attribute]) -> Vec<String> {
    attributes
        .iter()
        .filter_map(|attribute| {
            let name = attribute.path().segments.last()?.ident.to_string();
            if name != "cfg" && name != "cfg_attr" {
                return None;
            }
            let syn::Meta::List(list) = &attribute.meta else {
                return None;
            };
            let tokens = list
                .tokens
                .to_string()
                .split_whitespace()
                .collect::<String>();
            Some(format!("{name}({tokens})"))
        })
        .collect()
}

fn conformance_test_name(item: &syn::ItemMacro) -> Result<String> {
    let mut tokens = item.mac.tokens.clone().into_iter();
    let label = tokens
        .next()
        .context("conformance_test! is missing its name label")?;
    if label.to_string() != "name" {
        bail!("conformance_test! must begin with a name field");
    }
    let separator = tokens
        .next()
        .context("conformance_test! name is missing a colon")?;
    if separator.to_string() != ":" {
        bail!("conformance_test! name must be followed by a colon");
    }
    let literal = tokens
        .next()
        .context("conformance_test! is missing its name value")?;
    let name = syn::parse_str::<syn::LitStr>(&literal.to_string())
        .context("conformance_test! name must be a string literal")?;
    Ok(name.value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::collect::{SystemCommandRunner, SystemRepositoryReader, collect_workspace};
    use crate::context::model::TestDeclarationKind;
    use std::process::Command;

    #[test]
    fn source_inventory_retains_cfg_and_macro_declarations() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        std::fs::create_dir_all(root.join("sample/src"))
            .expect("sample source directory should be created");
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
resolver = "3"
members = ["sample"]
"#,
        )
        .expect("workspace manifest should be written");
        std::fs::write(
            root.join("sample/Cargo.toml"),
            r#"[package]
name = "inventory-sample"
version = "0.1.0"
edition = "2024"
"#,
        )
        .expect("package manifest should be written");
        std::fs::write(
            root.join("sample/src/lib.rs"),
            r#"#[cfg(unix)]
mod outer {
    #[cfg(feature = "alpha")]
    #[test]
    fn plain() {}

    mod inner {
        #[tokio::test]
        async fn async_case() {}
    }
}

#[cfg_attr(target_os = "linux", cfg(feature = "linux-only"))]
#[test]
fn cfg_attr_case() {}

crate::conformance_test! {
    name: "macro_case",
    adapter: "sample",
    capability: Run,
    category: Unit,
    |ctx| { ctx.result() }
}
"#,
        )
        .expect("Rust inventory fixture should be written");
        for args in [vec!["init", "--quiet"], vec!["add", "--all"]] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }

        let repository = SystemRepositoryReader;
        let workspace = collect_workspace(&SystemCommandRunner, &repository, root)
            .expect("workspace metadata should be collected");
        let package_id = workspace
            .packages
            .iter()
            .find(|package| package.name == "inventory-sample")
            .expect("fixture package should be present")
            .package_id
            .clone();
        let declarations = collect_source_tests(&repository, root, &workspace)
            .expect("source declarations should be collected");
        assert_eq!(declarations.len(), 4);

        let declaration = |name: &str| {
            declarations
                .iter()
                .find(|declaration| declaration.name == name)
                .expect("named declaration should be present")
        };
        let plain = declaration("plain");
        assert_eq!(plain.package_id, package_id);
        assert_eq!(plain.path.as_str(), "sample/src/lib.rs");
        assert_eq!(plain.module_path, ["outer"]);
        assert_eq!(
            plain.cfg_predicates,
            ["cfg(feature=\"alpha\")", "cfg(unix)"]
        );
        assert_eq!(plain.kind, TestDeclarationKind::Function);
        assert_eq!(
            plain.stable_id,
            format!("{}::sample/src/lib.rs::outer::plain::function", package_id)
        );

        let async_case = declaration("async_case");
        assert_eq!(async_case.module_path, ["outer", "inner"]);
        assert_eq!(async_case.cfg_predicates, ["cfg(unix)"]);
        assert_eq!(async_case.kind, TestDeclarationKind::Function);

        let cfg_attr = declaration("cfg_attr_case");
        assert!(cfg_attr.module_path.is_empty());
        assert_eq!(
            cfg_attr.cfg_predicates,
            ["cfg_attr(target_os=\"linux\",cfg(feature=\"linux-only\"))"]
        );

        let macro_case = declaration("macro_case");
        assert!(macro_case.module_path.is_empty());
        assert!(macro_case.cfg_predicates.is_empty());
        assert_eq!(macro_case.kind, TestDeclarationKind::MacroInvocation);
        assert_eq!(
            macro_case.stable_id,
            format!(
                "{}::sample/src/lib.rs::<root>::macro_case::macro_invocation",
                package_id
            )
        );

        assert!(
            declarations
                .windows(2)
                .all(|pair| pair[0].stable_id < pair[1].stable_id)
        );
        assert!(
            declarations
                .iter()
                .all(|declaration| declaration.path.as_str() == "sample/src/lib.rs")
        );
    }
}
