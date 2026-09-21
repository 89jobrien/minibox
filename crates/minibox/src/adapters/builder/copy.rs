//! Secure filesystem operations for Dockerfile `COPY` and local `ADD`.

use super::dockerignore::DockerIgnore;
use anyhow::{Context, Result, bail};
use minibox_core::image::dockerfile::CopyOwnership;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy)]
struct ResolvedOwnership {
    uid: u32,
    gid: u32,
}

pub(super) struct CopyRequest<'a> {
    pub(super) source_root: &'a Path,
    pub(super) sources: &'a [PathBuf],
    pub(super) destination: &'a Path,
    pub(super) workdir: &'a Path,
    pub(super) destination_rootfs: &'a Path,
    pub(super) layer_dir: &'a Path,
    pub(super) ignore: Option<&'a DockerIgnore>,
    pub(super) excluded_source: Option<&'a Path>,
    pub(super) chown: Option<&'a CopyOwnership>,
    pub(super) chmod: Option<u32>,
}

pub(super) fn copy_sources(request: &CopyRequest<'_>) -> Result<()> {
    if request.sources.is_empty() {
        bail!("COPY requires at least one source");
    }
    let destination = resolve_container_path(request.workdir, request.destination)?;
    reject_symlink_destination(request.destination_rootfs, &destination)?;
    let ownership = request
        .chown
        .map(|ownership| resolve_ownership(request.destination_rootfs, ownership))
        .transpose()?;
    let destination_is_directory = request.sources.len() > 1
        || request.destination.to_string_lossy().ends_with('/')
        || request.destination_rootfs.join(&destination).is_dir();
    if request.sources.len() > 1 && !destination_is_directory {
        bail!("COPY with multiple sources requires a directory destination");
    }

    for source in request.sources {
        let source_relative = validate_source_path(source)?;
        if request
            .excluded_source
            .is_some_and(|excluded| source_relative == excluded)
            || source_relative == Path::new(".dockerignore")
        {
            bail!(
                "COPY source is excluded from the build context: {}",
                source.display()
            );
        }
        validate_source_components(request.source_root, &source_relative)?;
        let source_path = request.source_root.join(&source_relative);
        let metadata = std::fs::symlink_metadata(&source_path)
            .with_context(|| format!("COPY source does not exist: {}", source.display()))?;
        if request
            .ignore
            .is_some_and(|ignore| ignore.is_ignored(&source_relative, metadata.is_dir()))
        {
            bail!(
                "COPY source is excluded by .dockerignore: {}",
                source.display()
            );
        }

        if metadata.is_dir() {
            let target = request.layer_dir.join(&destination);
            std::fs::create_dir_all(&target)
                .with_context(|| format!("create COPY destination {}", target.display()))?;
            copy_directory_contents(request, &source_path, &target, ownership)?;
            apply_attributes(&target, ownership, request.chmod, false)?;
        } else {
            let target = if destination_is_directory {
                let file_name = source_path
                    .file_name()
                    .context("COPY source has no file name")?;
                request.layer_dir.join(&destination).join(file_name)
            } else {
                request.layer_dir.join(&destination)
            };
            copy_node(request, &source_path, &target, &source_relative, ownership)?;
        }
    }
    Ok(())
}

fn copy_directory_contents(
    request: &CopyRequest<'_>,
    source: &Path,
    destination: &Path,
    ownership: Option<ResolvedOwnership>,
) -> Result<()> {
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("read COPY source directory {}", source.display()))?
    {
        let entry = entry.context("read COPY source entry")?;
        let source_path = entry.path();
        let relative = source_path
            .strip_prefix(request.source_root)
            .context("COPY source escaped source root")?;
        let metadata = std::fs::symlink_metadata(&source_path)
            .with_context(|| format!("inspect COPY source {}", source_path.display()))?;
        if request
            .excluded_source
            .is_some_and(|excluded| relative == excluded)
            || relative == Path::new(".dockerignore")
            || request
                .ignore
                .is_some_and(|ignore| ignore.is_ignored(relative, metadata.is_dir()))
        {
            continue;
        }
        copy_node(
            request,
            &source_path,
            &destination.join(entry.file_name()),
            relative,
            ownership,
        )?;
    }
    Ok(())
}

fn copy_node(
    request: &CopyRequest<'_>,
    source: &Path,
    destination: &Path,
    relative: &Path,
    ownership: Option<ResolvedOwnership>,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("inspect COPY source {}", source.display()))?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create COPY parent {}", parent.display()))?;
    }
    if metadata.file_type().is_symlink() {
        validate_context_symlink(request.source_root, source)?;
        remove_if_exists(destination)?;
        copy_symlink(source, destination)?;
        apply_attributes(destination, ownership, request.chmod, true)?;
    } else if metadata.is_dir() {
        if let Ok(destination_metadata) = std::fs::symlink_metadata(destination) {
            if destination_metadata.file_type().is_symlink() {
                bail!(
                    "COPY refuses to replace destination symlink: {}",
                    destination.display()
                );
            }
            if !destination_metadata.is_dir() {
                remove_if_exists(destination)?;
            }
        }
        std::fs::create_dir_all(destination)
            .with_context(|| format!("create COPY directory {}", destination.display()))?;
        copy_directory_contents(request, source, destination, ownership)?;
        apply_attributes(destination, ownership, request.chmod, false)?;
    } else if metadata.is_file() {
        reject_existing_symlink(destination)?;
        remove_if_exists(destination)?;
        std::fs::copy(source, destination)
            .with_context(|| format!("copy {} to {}", relative.display(), destination.display()))?;
        set_safe_source_permissions(destination, &metadata)?;
        apply_attributes(destination, ownership, request.chmod, false)?;
    } else {
        bail!(
            "COPY source has unsupported file type: {}",
            relative.display()
        );
    }
    Ok(())
}

fn validate_source_components(root: &Path, relative: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)
            .with_context(|| format!("inspect COPY source component {}", current.display()))?;
        if metadata.file_type().is_symlink() && index + 1 < component_count {
            bail!(
                "COPY source contains an intermediate symlink: {}",
                current.display()
            );
        }
    }
    Ok(())
}

fn validate_source_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!(
            "COPY source traversal: absolute context path {}",
            path.display()
        );
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => result.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("COPY source traversal is not allowed: {}", path.display())
            }
        }
    }
    Ok(result)
}

fn resolve_container_path(workdir: &Path, path: &Path) -> Result<PathBuf> {
    let mut parts = Vec::new();
    let combined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workdir.join(path)
    };
    for component in combined.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_os_string()),
            Component::ParentDir => {
                if parts.pop().is_none() {
                    bail!(
                        "COPY destination traversal is not allowed: {}",
                        path.display()
                    );
                }
            }
            Component::Prefix(_) => bail!("COPY destination uses an unsupported path prefix"),
        }
    }
    Ok(parts.into_iter().collect())
}

fn reject_symlink_destination(rootfs: &Path, destination: &Path) -> Result<()> {
    let mut current = rootfs.to_path_buf();
    for component in destination.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "COPY destination passes through unsafe symlink: {}",
                    current.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect COPY destination"),
        }
    }
    Ok(())
}

fn reject_existing_symlink(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        bail!(
            "COPY refuses to write through destination symlink: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_context_symlink(root: &Path, source: &Path) -> Result<()> {
    let target = std::fs::read_link(source)
        .with_context(|| format!("read context symlink {}", source.display()))?;
    if target.is_absolute() {
        bail!(
            "context symlink escapes build context: {}",
            source.display()
        );
    }
    let source_parent = source.parent().context("context symlink has no parent")?;
    let parent = source_parent
        .strip_prefix(root)
        .context("context symlink is outside source root")?;
    let mut depth = 0usize;
    for component in parent.join(&target).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                if depth == 0 {
                    bail!(
                        "context symlink escapes build context: {}",
                        source.display()
                    );
                }
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!(
                    "context symlink escapes build context: {}",
                    source.display()
                )
            }
        }
    }
    let candidate = source_parent.join(&target);
    if candidate.exists() {
        let canonical_root = root
            .canonicalize()
            .context("canonicalize COPY source root")?;
        let canonical_target = candidate
            .canonicalize()
            .with_context(|| format!("canonicalize symlink target {}", candidate.display()))?;
        if !canonical_target.starts_with(&canonical_root) {
            bail!(
                "context symlink target escapes build context: {}",
                source.display()
            );
        }
    }
    Ok(())
}

fn resolve_ownership(rootfs: &Path, ownership: &CopyOwnership) -> Result<ResolvedOwnership> {
    let (uid, default_gid) = if let Ok(uid) = ownership.user.parse::<u32>() {
        (uid, uid)
    } else {
        lookup_passwd(rootfs, &ownership.user)?
    };
    let gid = match ownership.group.as_deref() {
        None => default_gid,
        Some(group) => group
            .parse::<u32>()
            .map_or_else(|_| lookup_group(rootfs, group), Ok)?,
    };
    Ok(ResolvedOwnership { uid, gid })
}

fn lookup_passwd(rootfs: &Path, name: &str) -> Result<(u32, u32)> {
    let path = rootfs.join("etc/passwd");
    let contents = read_regular_file_no_follow(rootfs, &path)
        .with_context(|| format!("resolve --chown user {name:?} from {}", path.display()))?;
    for line in contents.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() == Some(&name) && fields.len() >= 4 {
            return Ok((fields[2].parse()?, fields[3].parse()?));
        }
    }
    bail!("--chown user not found in destination stage: {name}")
}

fn lookup_group(rootfs: &Path, name: &str) -> Result<u32> {
    let path = rootfs.join("etc/group");
    let contents = read_regular_file_no_follow(rootfs, &path)
        .with_context(|| format!("resolve --chown group {name:?} from {}", path.display()))?;
    for line in contents.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first() == Some(&name) && fields.len() >= 3 {
            return Ok(fields[2].parse()?);
        }
    }
    bail!("--chown group not found in destination stage: {name}")
}

#[cfg(unix)]
fn read_regular_file_no_follow(rootfs: &Path, path: &Path) -> Result<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let relative = path
        .strip_prefix(rootfs)
        .context("ownership database path escaped stage root")?;
    validate_source_components(rootfs, relative)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!(
            "ownership database must be a regular no-follow file: {}",
            path.display()
        );
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("open ownership database {}", path.display()))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

#[cfg(not(unix))]
fn read_regular_file_no_follow(_rootfs: &Path, path: &Path) -> Result<String> {
    bail!(
        "named --chown resolution cannot be applied safely on this platform: {}",
        path.display()
    )
}

fn apply_attributes(
    path: &Path,
    ownership: Option<ResolvedOwnership>,
    chmod: Option<u32>,
    is_symlink: bool,
) -> Result<()> {
    if let Some(mode) = chmod
        && !is_symlink
    {
        set_mode(path, mode)?;
    }
    if let Some(ownership) = ownership {
        set_ownership(path, ownership)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("apply --chmod to {}", path.display()))
}

#[cfg(unix)]
fn set_safe_source_permissions(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(metadata.mode() & 0o777),
    )
    .with_context(|| format!("mask special permission bits on {}", path.display()))
}

#[cfg(not(unix))]
fn set_safe_source_permissions(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    std::fs::set_permissions(path, metadata.permissions())
        .with_context(|| format!("preserve permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(path: &Path, _mode: u32) -> Result<()> {
    bail!(
        "--chmod cannot be applied correctly on this platform: {}",
        path.display()
    )
}

#[cfg(unix)]
fn set_ownership(path: &Path, ownership: ResolvedOwnership) -> Result<()> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{Gid, Uid, fchownat};
    fchownat(
        None,
        path,
        Some(Uid::from_raw(ownership.uid)),
        Some(Gid::from_raw(ownership.gid)),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .with_context(|| {
        format!(
            "apply --chown {}:{} to {}; active platform may lack required privilege",
            ownership.uid,
            ownership.gid,
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn set_ownership(path: &Path, _ownership: ResolvedOwnership) -> Result<()> {
    bail!(
        "--chown cannot be applied correctly on this platform: {}",
        path.display()
    )
}

fn remove_if_exists(path: &Path) -> Result<()> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .with_context(|| format!("remove existing path {}", path.display()))
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = std::fs::read_link(source)?;
    std::os::unix::fs::symlink(target, destination)
        .with_context(|| format!("preserve symlink {}", source.display()))
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, _destination: &Path) -> Result<()> {
    bail!(
        "preserving COPY symlinks is unsupported on this platform: {}",
        source.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn context_symlink_escape_is_rejected_without_following_link() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(&rootfs).expect("create rootfs");
        std::os::unix::fs::symlink("../../outside", context.join("escape"))
            .expect("create symlink");
        let sources = vec![PathBuf::from("escape")];
        let result = copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/escape"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: None,
            chmod: None,
        });
        assert!(result.is_err());
        assert!(!layer.join("escape").exists());
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_source_symlink_is_rejected_even_when_target_is_internal() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(context.join("real")).expect("create real source");
        std::fs::create_dir_all(&rootfs).expect("create rootfs");
        std::fs::write(context.join("real/file"), "contents").expect("write source");
        std::os::unix::fs::symlink("real", context.join("linked"))
            .expect("create intermediate symlink");
        let sources = vec![PathBuf::from("linked/file")];

        let error = copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/file"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: None,
            chmod: None,
        })
        .expect_err("intermediate source symlink must be rejected");
        assert!(error.to_string().contains("intermediate symlink"));
    }

    #[test]
    fn destination_traversal_is_rejected() {
        let error = resolve_container_path(Path::new("/"), Path::new("../../host"))
            .expect_err("traversal must fail");
        assert!(error.to_string().contains("traversal"));
    }

    #[cfg(unix)]
    #[test]
    fn safe_symlink_is_preserved_without_following() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(&rootfs).expect("create rootfs");
        std::fs::write(context.join("target"), "contents").expect("write target");
        std::os::unix::fs::symlink("target", context.join("link")).expect("create symlink");
        let sources = vec![PathBuf::from("link")];

        copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/link"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: None,
            chmod: None,
        })
        .expect("copy safe symlink");

        assert!(
            std::fs::symlink_metadata(layer.join("link"))
                .expect("inspect copied link")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(layer.join("link")).expect("read copied link"),
            PathBuf::from("target")
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_rejects_destination_symlink() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(&rootfs).expect("create rootfs");
        std::fs::write(context.join("file"), "contents").expect("write source");
        std::os::unix::fs::symlink("elsewhere", rootfs.join("unsafe"))
            .expect("create destination symlink");
        let sources = vec![PathBuf::from("file")];
        let result = copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/unsafe/file"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: None,
            chmod: None,
        });
        assert!(result.is_err());
        assert!(
            result
                .expect_err("unsafe symlink must fail")
                .to_string()
                .contains("symlink")
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_applies_octal_mode_and_resolves_named_ownership() {
        use std::os::unix::fs::MetadataExt;

        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(rootfs.join("etc")).expect("create rootfs etc");
        std::fs::write(context.join("file"), "contents").expect("write source");
        let current = std::fs::metadata(&context).expect("context metadata");
        std::fs::write(
            rootfs.join("etc/passwd"),
            format!("builder:x:{}:{}::/:/bin/sh\n", current.uid(), current.gid()),
        )
        .expect("write passwd");
        std::fs::write(
            rootfs.join("etc/group"),
            format!("builders:x:{}:\n", current.gid()),
        )
        .expect("write group");
        let sources = vec![PathBuf::from("file")];
        let ownership = CopyOwnership {
            user: "builder".to_string(),
            group: Some("builders".to_string()),
        };

        copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/file"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: Some(&ownership),
            chmod: Some(0o640),
        })
        .expect("copy with ownership and mode");

        let metadata = std::fs::metadata(layer.join("file")).expect("copied metadata");
        assert_eq!(metadata.mode() & 0o777, 0o640);
        assert_eq!(metadata.uid(), current.uid());
        assert_eq!(metadata.gid(), current.gid());
    }

    #[cfg(unix)]
    #[test]
    fn named_chown_rejects_symlinked_passwd_database() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(rootfs.join("etc")).expect("create rootfs etc");
        std::fs::write(context.join("file"), "contents").expect("write source");
        std::fs::write(rootfs.join("passwd-real"), "builder:x:1:1::/:/bin/sh\n")
            .expect("write passwd target");
        std::os::unix::fs::symlink("../passwd-real", rootfs.join("etc/passwd"))
            .expect("symlink passwd");
        let sources = vec![PathBuf::from("file")];
        let ownership = CopyOwnership {
            user: "builder".to_string(),
            group: None,
        };

        let error = copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/file"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: Some(&ownership),
            chmod: None,
        })
        .expect_err("symlinked passwd must be rejected");
        assert!(
            format!("{error:#}").contains("regular no-follow"),
            "error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_masks_setuid_setgid_and_sticky_bits() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        let layer = temp.path().join("layer");
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&context).expect("create context");
        std::fs::create_dir_all(&rootfs).expect("create rootfs");
        let source = context.join("special");
        std::fs::write(&source, "contents").expect("write source");
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o7755))
            .expect("set source mode");
        let sources = vec![PathBuf::from("special")];

        copy_sources(&CopyRequest {
            source_root: &context,
            sources: &sources,
            destination: Path::new("/special"),
            workdir: Path::new("/"),
            destination_rootfs: &rootfs,
            layer_dir: &layer,
            ignore: None,
            excluded_source: None,
            chown: None,
            chmod: None,
        })
        .expect("copy special-mode file");

        let mode = std::fs::metadata(layer.join("special"))
            .expect("copied metadata")
            .mode();
        assert_eq!(mode & 0o7000, 0);
        assert_eq!(mode & 0o777, 0o755);
    }
}
