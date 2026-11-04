//! `cargo xtask build-vm-image` — build the EFI system partition the `vz` adapter boots.
//!
//! `VZEFIBootLoader` takes only a variable store, so the guest kernel command line cannot be
//! passed through the framework. The ESP therefore carries a boot manager (GRUB), a
//! `grub.cfg` with the command line rendered from
//! [`minibox_core::vm_boot::kernel_cmdline`], the Alpine kernel, and an initramfs that can
//! mount a virtiofs root and bring up the vsock transport the agent needs.
//!
//! Artifacts are pinned by release and sha256; nothing is fetched "latest". Output is
//! deterministic: fixed volume id/label, fixed 1980 timestamps, files written in a fixed
//! order, and a reproducible gzip header.
//!
//! See `docs/designs/2026-10-22-vz-efi-boot-design.md`.

use anyhow::{Context, Result, bail};
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use minibox_core::vm_boot;

/// Alpine release the boot assets come from.
pub const ALPINE_RELEASE: &str = "3.22.7";
/// Alpine CPU architecture the boot assets target.
pub const ALPINE_ARCH: &str = "aarch64";
/// Filename of the pinned Alpine virt ISO.
pub const ISO_FILE: &str = "alpine-virt-3.22.7-aarch64.iso";
/// Download URL of the pinned Alpine virt ISO.
pub const ISO_URL: &str =
    "https://dl-cdn.alpinelinux.org/alpine/v3.22/releases/aarch64/alpine-virt-3.22.7-aarch64.iso";
/// sha256 of the pinned Alpine virt ISO.
pub const ISO_SHA256: &str = "3e37f238f6317ef56a12f4cff5c4265f99d25785945f777b19d41b35a7fabbc7";

/// Alpine `linux-virt` package that supplies the vsock modules.
pub const LINUX_VIRT_PACKAGE: &str = "linux-virt-6.12.114-r0.apk";
/// Download URL of the pinned `linux-virt` package.
pub const LINUX_VIRT_URL: &str =
    "https://dl-cdn.alpinelinux.org/alpine/v3.22/main/aarch64/linux-virt-6.12.114-r0.apk";
/// sha256 of the pinned `linux-virt` package.
pub const LINUX_VIRT_SHA256: &str =
    "345b2d561443cd8c978cbcc0f36f2374b3ef702debb747221384b4ddc07d6a77";
/// `uname -r` of the pinned kernel, i.e. the `/lib/modules` directory name.
///
/// Re-exported from [`vm_boot::KERNEL_RELEASE`] rather than redefined, so this builder and
/// the guest init script (`crates/macbox/src/vz/agent_init.rs`) cannot silently diverge.
pub use vm_boot::KERNEL_RELEASE;

/// Size of the FAT32 ESP image in bytes.
pub const ESP_SIZE_BYTES: u64 = 128 * 1024 * 1024;
/// Fixed FAT volume id, so repeated builds produce identical images.
pub const ESP_VOLUME_ID: u32 = 0x4d42_5845;
/// Fixed FAT volume label (`MINIBOX` padded to 11 bytes).
pub const ESP_VOLUME_LABEL: [u8; 11] = *b"MINIBOX    ";
/// Fixed timestamp written for every file and directory (DOS epoch).
pub const ESP_TIMESTAMP: fatfs::DateTime = fatfs::DateTime {
    date: fatfs::Date {
        year: 1980,
        month: 1,
        day: 1,
    },
    time: fatfs::Time {
        hour: 0,
        min: 0,
        sec: 0,
        millis: 0,
    },
};

/// Manifest written next to the ESP so later runs can skip rebuilding unchanged inputs.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Manifest schema version.
pub const MANIFEST_FORMAT: u32 = 1;

/// One module appended to the Alpine initramfs.
#[derive(Debug, Clone, Copy)]
pub struct ModuleSpec {
    /// Module name as `modprobe` receives it.
    pub name: &'static str,
    /// Path relative to `/lib/modules/<release>/` inside the initramfs.
    pub path: &'static str,
    /// Modules this one depends on, in load order.
    pub depends: &'static [&'static str],
}

/// Modules appended to the Alpine initramfs for the guest agent's vsock transport.
pub const VSOCK_MODULES: &[ModuleSpec] = &[
    ModuleSpec {
        name: "vsock",
        path: "kernel/net/vmw_vsock/vsock.ko",
        depends: &[],
    },
    ModuleSpec {
        name: "vmw_vsock_virtio_transport_common",
        path: "kernel/net/vmw_vsock/vmw_vsock_virtio_transport_common.ko",
        depends: &["vsock"],
    },
    ModuleSpec {
        name: "vmw_vsock_virtio_transport",
        path: "kernel/net/vmw_vsock/vmw_vsock_virtio_transport.ko",
        depends: &["vsock", "vmw_vsock_virtio_transport_common"],
    },
];

/// Modules appended to the Alpine initramfs for the guest's network device and
/// `udhcpc`'s DHCP transport. `virtio_net` is the driver for the
/// `VZVirtioNetworkDeviceConfiguration` NIC itself -- without it `eth0` never
/// exists (`ip: can't find device 'eth0'`), so nothing downstream (DHCP, DNS,
/// image pulls) can work. `virtio_net` itself depends on `net_failover`, which
/// depends on `failover` (confirmed via each `.ko`'s embedded `depends=` modinfo
/// string) -- `insmod virtio_net.ko` with either missing fails with an unresolved
/// symbol error that the guest's `2>/dev/null || true` swallows, so `eth0` silently
/// never appears even though the kernel cmdline's `modules=` and the rcS insmod
/// line both name `virtio_net`. `udhcpc` separately opens an `AF_PACKET` socket to
/// send/receive raw DHCP frames. All ship as loadable modules on the `linux-virt`
/// kernel, not built in.
pub const NET_MODULES: &[ModuleSpec] = &[
    ModuleSpec {
        name: "failover",
        path: "kernel/net/core/failover.ko",
        depends: &[],
    },
    ModuleSpec {
        name: "net_failover",
        path: "kernel/drivers/net/net_failover.ko",
        depends: &["failover"],
    },
    ModuleSpec {
        name: "virtio_net",
        path: "kernel/drivers/net/virtio_net.ko",
        depends: &["failover", "net_failover"],
    },
    ModuleSpec {
        name: "af_packet",
        path: "kernel/net/packet/af_packet.ko",
        depends: &[],
    },
];

/// Modules appended to the Alpine initramfs for container overlay rootfs mounts.
/// `mount -t overlay` returns `ENODEV` without this; the `linux-virt` kernel ships
/// overlayfs as a loadable module, not built in.
pub const FS_MODULES: &[ModuleSpec] = &[ModuleSpec {
    name: "overlay",
    path: "kernel/fs/overlayfs/overlay.ko",
    depends: &[],
}];

/// Inputs and outputs recorded in [`MANIFEST_FILE`].
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct Manifest {
    /// Manifest schema version.
    pub format: u32,
    /// Alpine release the assets were taken from.
    pub alpine_release: String,
    /// sha256 of the ISO used for the boot manager, initramfs, and kernel.
    pub iso_sha256: String,
    /// sha256 of the `linux-virt` package used for the vsock modules.
    pub linux_virt_sha256: String,
    /// Kernel command line rendered into `grub.cfg`.
    pub kernel_cmdline: String,
    /// sha256 of each file written into the ESP, keyed by ESP path.
    pub esp_files: Vec<EspFile>,
    /// sha256 of the finished ESP image.
    pub esp_sha256: String,
}

/// One file inside the ESP, as recorded in the manifest.
#[derive(Debug, Serialize, serde::Deserialize)]
pub struct EspFile {
    /// Path inside the ESP.
    pub path: String,
    /// Number of bytes written.
    pub size: u64,
    /// sha256 of the file contents.
    pub sha256: String,
}

/// A single `newc` cpio entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpioEntry {
    /// Path relative to the initramfs root.
    pub name: String,
    /// POSIX mode bits (`S_IFDIR | 0o755`, `S_IFREG | 0o644`, ...).
    pub mode: u32,
    /// Entry contents (empty for directories).
    pub data: Vec<u8>,
}

const CPIO_HEADER_LEN: usize = 110;
const CPIO_TRAILER: &str = "TRAILER!!!";
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const MODE_DIR: u32 = S_IFDIR | 0o755;
const MODE_FILE: u32 = S_IFREG | 0o644;

const fn align4(value: usize) -> usize {
    value.div_ceil(4) * 4
}

/// Parse every `newc` archive in `bytes`, including concatenated archives.
///
/// The kernel unpacks concatenated cpio archives in order, which is what lets the builder
/// append modules (and a rewritten `modules.dep`) to Alpine's image without unpacking and
/// repacking the whole initramfs.
pub fn parse_newc(bytes: &[u8]) -> Result<Vec<CpioEntry>> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while let Some(start) = find_magic(bytes, pos) {
        let field = |index: usize| -> Result<usize> {
            let offset = start + 6 + index * 8;
            let raw = bytes
                .get(offset..offset + 8)
                .context("truncated cpio header")?;
            let text = std::str::from_utf8(raw).context("cpio header is not ASCII")?;
            usize::from_str_radix(text, 16)
                .with_context(|| format!("invalid cpio header field {text:?}"))
        };

        let mode = u32::try_from(field(1)?).context("cpio mode out of range")?;
        let size = field(6)?;
        let namesize = field(11)?;
        let name_start = start + CPIO_HEADER_LEN;
        let name_end = name_start
            .checked_add(namesize)
            .context("cpio name length overflow")?;
        let raw_name = bytes
            .get(name_start..name_end)
            .context("truncated cpio name")?;
        let raw_name = raw_name.strip_suffix(&[0]).unwrap_or(raw_name);
        let name = String::from_utf8_lossy(raw_name).into_owned();

        let data_start = align4(name_end);
        let data_end = data_start
            .checked_add(size)
            .context("cpio data length overflow")?;
        let data = bytes
            .get(data_start..data_end)
            .context("truncated cpio data")?;

        if name == CPIO_TRAILER {
            pos = data_end;
            continue;
        }
        entries.push(CpioEntry {
            name,
            mode,
            data: data.to_vec(),
        });
        pos = data_end;
    }
    Ok(entries)
}

fn find_magic(bytes: &[u8], from: usize) -> Option<usize> {
    if from >= bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(6)
        .position(|window| window == b"070701" || window == b"070702")
        .map(|offset| from + offset)
}

/// Serialize entries as a single `newc` archive with fixed metadata.
pub fn write_newc(entries: &[CpioEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let ino = u32::try_from(index + 1).unwrap_or(u32::MAX);
        write_newc_entry(&mut out, entry, ino);
    }
    write_newc_entry(
        &mut out,
        &CpioEntry {
            name: CPIO_TRAILER.to_string(),
            mode: 0,
            data: Vec::new(),
        },
        0,
    );
    out
}

fn write_newc_entry(out: &mut Vec<u8>, entry: &CpioEntry, ino: u32) {
    let name_len = entry.name.len() + 1;
    let fields = [
        ino,
        entry.mode,
        0, // uid
        0, // gid
        1, // nlink
        0, // mtime, fixed for reproducibility
        u32::try_from(entry.data.len()).unwrap_or(u32::MAX),
        0, // devmajor
        0, // devminor
        0, // rdevmajor
        0, // rdevminor
        u32::try_from(name_len).unwrap_or(u32::MAX),
        0, // check
    ];
    out.extend_from_slice(b"070701");
    for field in fields {
        out.extend_from_slice(format!("{field:08x}").as_bytes());
    }
    out.extend_from_slice(entry.name.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(&entry.data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn gzip_decode(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .with_context(|| format!("decompressing {}", path.display()))?;
    Ok(out)
}

fn gzip_encode(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).context("gzip compressing")?;
    encoder.finish().context("finishing gzip stream")
}

/// Append the vsock modules and a merged `modules.dep` to Alpine's initramfs.
///
/// `alpine_cpio` is the decompressed initramfs; `modules` maps module file name to its
/// decompressed `.ko` bytes.
pub fn augment_initramfs(alpine_cpio: &[u8], modules: &[(ModuleSpec, Vec<u8>)]) -> Result<Vec<u8>> {
    let existing = parse_newc(alpine_cpio)?;
    let module_dir = format!("lib/modules/{KERNEL_RELEASE}");
    let dep_path = format!("{module_dir}/modules.dep");

    let original_dep = existing
        .iter()
        .find(|entry| entry.name == dep_path)
        .with_context(|| format!("initramfs has no {dep_path}"))?;
    let mut dep_text = String::from_utf8(original_dep.data.clone())
        .with_context(|| format!("{dep_path} is not UTF-8"))?;
    if !dep_text.ends_with('\n') {
        dep_text.push('\n');
    }

    let mut entries = Vec::new();
    let mut directories = vec![
        "lib".to_string(),
        "lib/modules".to_string(),
        module_dir.clone(),
    ];
    for (spec, data) in modules {
        let destination = format!("{module_dir}/{}", spec.path);
        if existing.iter().any(|entry| entry.name == destination) {
            println!("  skip    {destination} (already in initramfs)");
            continue;
        }
        let depends = spec
            .depends
            .iter()
            .map(|name| dependency_path(modules, name))
            .collect::<Result<Vec<_>>>()?;
        let mut line = format!("{}:", spec.path);
        for dep in &depends {
            line.push(' ');
            line.push_str(dep);
        }
        line.push('\n');
        dep_text.push_str(&line);

        let mut parent = Path::new(&destination).parent();
        let mut chain: Vec<String> = Vec::new();
        while let Some(dir) = parent {
            let dir_str = dir.to_string_lossy().into_owned();
            if dir_str.is_empty() {
                break;
            }
            if directories.contains(&dir_str) {
                break;
            }
            chain.push(dir_str);
            if let Some(p) = Path::new(chain.last().expect("chain not empty")).parent() {
                parent = Some(p);
            } else {
                break;
            }
        }
        for dir in chain.into_iter().rev() {
            if !directories.contains(&dir) {
                directories.push(dir);
            }
        }
        entries.push(CpioEntry {
            name: destination,
            mode: MODE_FILE,
            data: data.clone(),
        });
    }

    directories.sort();
    directories.dedup();
    let mut segment: Vec<CpioEntry> = directories
        .into_iter()
        .map(|name| CpioEntry {
            name,
            mode: MODE_DIR,
            data: Vec::new(),
        })
        .collect();
    segment.extend(entries);
    segment.push(CpioEntry {
        name: dep_path,
        mode: MODE_FILE,
        data: dep_text.into_bytes(),
    });
    segment.sort_by(|a, b| a.name.cmp(&b.name));

    let mut combined = alpine_cpio.to_vec();
    combined.extend_from_slice(&write_newc(&segment));
    gzip_encode(&combined)
}

fn dependency_path<'a>(modules: &'a [(ModuleSpec, Vec<u8>)], name: &str) -> Result<&'a str> {
    modules
        .iter()
        .find(|(spec, _)| spec.name == name)
        .map(|(spec, _)| spec.path)
        .with_context(|| format!("module dependency {name:?} is not in the appended set"))
}

/// Render the GRUB configuration that boots the guest kernel with `cmdline`.
#[must_use]
pub fn render_grub_cfg(cmdline: &str) -> String {
    format!(
        "set timeout=0\n\
         set default=0\n\
         menuentry \"minibox\" {{\n\
         \x20 linux /{} {cmdline}\n\
         \x20 initrd /{}\n\
         }}\n",
        vm_boot::ESP_KERNEL,
        vm_boot::ESP_INITRAMFS
    )
}

/// Build one file inside the ESP.
#[derive(Debug)]
pub struct EspFilePlan {
    /// Path inside the ESP.
    pub path: String,
    /// File contents.
    pub data: Vec<u8>,
}

/// Write a FAT32 image containing `files`, with deterministic metadata.
pub fn build_fat_image(
    dest: &Path,
    size_bytes: u64,
    files: &[EspFilePlan],
) -> Result<Vec<EspFile>> {
    {
        let file = File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
        file.set_len(size_bytes)
            .with_context(|| format!("sizing {}", dest.display()))?;
    }
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(dest)
        .with_context(|| format!("reopening {}", dest.display()))?;
    let options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat32)
        .volume_id(ESP_VOLUME_ID)
        .volume_label(ESP_VOLUME_LABEL);
    fatfs::format_volume(&mut file, options).context("formatting FAT32 ESP")?;

    let esp_files = {
        let fs = FileSystem::new(&file, FsOptions::new()).context("opening FAT32 ESP")?;
        let mut esp_files = Vec::new();
        {
            let root = fs.root_dir();

            for plan in files {
                let mut components: Vec<&str> = plan.path.split('/').collect();
                let file_name = components.pop().context("ESP path has no file name")?;
                let mut dir = root.clone();
                for component in components {
                    dir = match dir.open_dir(component) {
                        Ok(existing) => existing,
                        Err(_) => dir
                            .create_dir(component)
                            .with_context(|| format!("creating ESP directory {component}"))?,
                    };
                }
                let mut handle = dir
                    .create_file(file_name)
                    .with_context(|| format!("creating ESP file {}", plan.path))?;
                handle
                    .write_all(&plan.data)
                    .with_context(|| format!("writing ESP file {}", plan.path))?;
                #[allow(deprecated)]
                handle.set_created(ESP_TIMESTAMP);
                #[allow(deprecated)]
                handle.set_modified(ESP_TIMESTAMP);
                #[allow(deprecated)]
                handle.set_accessed(ESP_TIMESTAMP.date);
                handle.flush().context("flushing ESP file")?;

                esp_files.push(EspFile {
                    path: plan.path.clone(),
                    size: u64::try_from(plan.data.len()).unwrap_or(u64::MAX),
                    sha256: sha256_hex(&plan.data),
                });
            }
        }
        fs.unmount().context("unmounting FAT32 ESP")?;
        esp_files
    };
    Ok(esp_files)
}

/// Compute the lowercase hex sha256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).context("reading file")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn ensure_download(url: &str, dest: &Path, expected_sha256: &str) -> Result<()> {
    if dest.exists() {
        let actual = sha256_file(dest)?;
        if actual == expected_sha256 {
            println!("  cached  {}", dest.display());
            return Ok(());
        }
        println!(
            "  refresh {} (sha256 {actual} does not match pinned {expected_sha256})",
            dest.display()
        );
        fs::remove_file(dest).with_context(|| format!("removing {}", dest.display()))?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    println!("  fetch   {url}");
    let status = Command::new("curl")
        .args(["--silent", "--show-error", "--location", "--fail", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .context("running curl")?;
    if !status.success() {
        bail!("curl failed for {url}");
    }
    let actual = sha256_file(dest)?;
    if actual != expected_sha256 {
        bail!("sha256 mismatch for {url}: expected {expected_sha256}, got {actual}");
    }
    Ok(())
}

fn extract_member(archive: &Path, member: &str, dest: &Path, gzipped: bool) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let staging = dest
        .parent()
        .context("extraction destination has no parent")?;
    let mut command = Command::new("tar");
    command
        .arg("-x")
        .arg("-f")
        .arg(archive)
        .arg("-C")
        .arg(staging);
    if gzipped {
        command.arg("-z");
    }
    command.arg(member);
    let output = command
        .output()
        .with_context(|| format!("running tar for {member}"))?;
    if !output.status.success() {
        bail!(
            "tar failed to extract {member} from {}: {}",
            archive.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let extracted = staging.join(member);
    if extracted != dest {
        fs::rename(&extracted, dest)
            .with_context(|| format!("moving extracted {member} to {}", dest.display()))?;
    }
    Ok(())
}

fn read_modules(
    apk: &Path,
    staging: &Path,
    specs: &[ModuleSpec],
) -> Result<Vec<(ModuleSpec, Vec<u8>)>> {
    let mut modules = Vec::with_capacity(specs.len());
    for spec in specs {
        let source = format!("lib/modules/{KERNEL_RELEASE}/{}.gz", spec.path);
        let extracted = staging.join(&source);
        extract_member(apk, &source, &extracted, true)?;
        let data = gzip_decode(&extracted)?;
        modules.push((*spec, data));
    }
    Ok(modules)
}

/// Options for [`run`].
#[derive(Debug, Clone)]
pub struct Options {
    /// VM asset directory (defaults to `~/.minibox/vm`).
    pub vm_dir: PathBuf,
    /// Rebuild even when the manifest matches the inputs.
    pub force: bool,
}

impl Options {
    /// Parse `build-vm-image` arguments (`--force`, `--vm-dir <dir>`).
    pub fn from_args(args: &[String]) -> Result<Self> {
        let mut vm_dir = dirs::home_dir()
            .context("resolving the home directory")?
            .join(".minibox")
            .join("vm");
        let mut force = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--force" => force = true,
                "--vm-dir" => {
                    let value = iter.next().context("--vm-dir needs a directory path")?;
                    vm_dir = PathBuf::from(value);
                }
                other => bail!("unexpected build-vm-image argument {other:?}"),
            }
        }
        Ok(Self { vm_dir, force })
    }
}

/// `AF_VSOCK` port the guest agent listens on. Must match
/// `crates/macbox/src/vz/vsock.rs::AGENT_PORT` (the host side of the same
/// channel) and `crates/miniboxd/src/listener.rs::VsockServerListener`.
///
/// Re-exported from [`vm_boot::VSOCK_AGENT_PORT`] rather than redefined, so this builder
/// and the guest init script (`crates/macbox/src/vz/agent_init.rs`) cannot silently
/// diverge.
use vm_boot::VSOCK_AGENT_PORT;

/// Target triple the guest agent is cross-compiled for (Apple Silicon hosts
/// run an `aarch64` VZ guest).
const AGENT_TARGET_TRIPLE: &str = "aarch64-unknown-linux-musl";

/// Generate `/etc/inittab` content that boots the guest agent (`miniboxd`
/// itself, staged as `/sbin/minibox-agent` and started with
/// `MINIBOX_VSOCK_PORT` set) via busybox init.
///
/// Intentionally duplicated from the shape of `macbox::vz::agent_init`
/// rather than shared via a dependency: `xtask` does not depend on `macbox`,
/// which pulls in macOS-only Virtualization.framework bindings behind the
/// `vz` feature.
///
/// `MINIBOX_ALLOW_PRIVILEGED`/`MINIBOX_ALLOW_BIND_MOUNTS` are set because the
/// guest agent IS the thing the vz isolation tests exercise -- they boot
/// privileged containers inside this VM specifically to assert namespace
/// isolation. The policy gate's default-deny is a host-daemon protection
/// against an untrusted local client; it does not apply here, since the only
/// client that can reach this agent is the macOS host over vsock (see
/// `VsockServerListener`'s doc comment in `crates/miniboxd/src/listener.rs`
/// for why that's already the trust boundary).
fn generate_inittab() -> String {
    format!(
        "::sysinit:/etc/init.d/rcS\n::once:/bin/sh -c \"MINIBOX_VSOCK_PORT={VSOCK_AGENT_PORT} MINIBOX_ALLOW_PRIVILEGED=1 MINIBOX_ALLOW_BIND_MOUNTS=1 exec /sbin/minibox-agent\"\n::ctrlaltdel:/sbin/reboot\n::shutdown:/bin/umount -a -r\n"
    )
}

/// Generate `/etc/init.d/rcS` content: minimal system prep before the agent starts.
fn generate_rc_local() -> String {
    format!(
        r"#!/bin/sh
# Minimal system initialization for the minibox vz-adapter guest agent
set -e

# The virtiofs root is shared read-write from the host (see build_virtiofs()
# in crates/macbox/src/vz/vm.rs, called with read_only=false for ROOTFS_TAG)
# and the kernel cmdline passes `rw` (see minibox_core::vm_boot::kernel_cmdline),
# but the initramfs that hands off to this script still mounts the virtiofs
# root read-only before switch_root. Force it rw before anything below tries
# to create a directory on it.
mount -o remount,rw / 2>/dev/null || true

mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# cgroup v2 unified hierarchy. miniboxd creates container cgroups under
# /sys/fs/cgroup/minibox; without this mount that mkdir fails with ENOENT
# because /sys/fs/cgroup is never populated by the sysfs mount above --
# cgroupfs is a separate filesystem type that must be mounted explicitly.
mkdir -p /sys/fs/cgroup
mount -t cgroup2 cgroup2 /sys/fs/cgroup 2>/dev/null || true

# virtiofs (FUSE-backed) does not support AF_UNIX bind(2) on its shared
# directories -- miniboxd's own control socket at /run/minibox/miniboxd.sock
# must live on a real Linux filesystem, so give it a tmpfs.
mkdir -p /run
mount -t tmpfs tmpfs /run 2>/dev/null || true
mkdir -p /run/minibox

# The `modules=` kernel cmdline param asks the initramfs to modprobe these
# before switch_root, but that has been observed not to actually load them.
# Load explicitly, in dependency order, so AF_VSOCK is available to the agent.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vsock.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vmw_vsock_virtio_transport_common.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vmw_vsock_virtio_transport.ko 2>/dev/null || true

# virtio_net is the driver for the guest NIC itself (VZVirtioNetworkDeviceConfiguration);
# without it eth0 never exists at all. It depends on net_failover, which depends on
# failover -- load those first or insmod fails on an unresolved symbol (silently, since
# errors are discarded below) and eth0 never appears. udhcpc separately opens an
# AF_PACKET socket for raw DHCP frames. Same modules= gap as above.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/core/failover.ko && echo OK:failover || echo FAIL:failover
insmod /lib/modules/{KERNEL_RELEASE}/kernel/drivers/net/net_failover.ko && echo OK:net_failover || echo FAIL:net_failover
insmod /lib/modules/{KERNEL_RELEASE}/kernel/drivers/net/virtio_net.ko && echo OK:virtio_net || echo FAIL:virtio_net
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/packet/af_packet.ko && echo OK:af_packet || echo FAIL:af_packet
lsmod

# overlay rootfs mounts need this; same modules= gap as above.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/fs/overlayfs/overlay.ko 2>/dev/null || true

mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs minibox-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs minibox-containers /var/lib/minibox/containers 2>/dev/null || true

ip link set lo up 2>/dev/null || true

# Guest network device (VZVirtioNetworkDeviceConfiguration + NAT attachment, see
# crates/macbox/src/vz/vm.rs). udhcpc + its default script (both pre-provisioned
# in this rootfs) write /etc/resolv.conf from the DHCP-offered nameserver.
ip link set eth0 up 2>/dev/null || true

# eth0 can still be negotiating (or the host's VZNATNetworkDeviceAttachment
# DHCP server not yet ready) the instant this script runs, which makes a
# single udhcpc -n attempt fail with no lease and never retries, leaving the
# guest with no route/DNS for the rest of boot. Retry briefly instead of
# treating link-not-ready as permanent failure.
for _ in 1 2 3 4 5 6 7 8 9 10; do
    udhcpc -i eth0 -n -q 2>/dev/null && break
    sleep 1
done

echo '-- net diag --'
ip link show || true
ip addr show eth0 2>&1 || true
ip route 2>&1 || true
cat /etc/resolv.conf 2>&1 || true
echo '-- virtio bus devices --'
ls /sys/bus/virtio/devices 2>&1 || true
for d in /sys/bus/virtio/devices/*; do
    echo $d: || true
    cat $d/device 2>&1 || true
    cat $d/driver_override 2>&1 || true
    ls $d/driver 2>&1 || true
done
echo '-- dmesg --'
dmesg 2>&1 | grep -iE 'virtio|failover|eth0|net_fail' | tail -40 || true
echo '-- end net diag --'

hostname minibox-vm 2>/dev/null || true
"
    )
}

/// Install busybox init files into `rootfs_dir/etc/`.
fn install_init_files(rootfs_dir: &Path) -> Result<()> {
    let etc = rootfs_dir.join("etc");
    fs::create_dir_all(&etc).with_context(|| format!("creating {}", etc.display()))?;

    let inittab = etc.join("inittab");
    fs::write(&inittab, generate_inittab())
        .with_context(|| format!("writing {}", inittab.display()))?;

    let initd = etc.join("init.d");
    fs::create_dir_all(&initd).with_context(|| format!("creating {}", initd.display()))?;
    let rcs = initd.join("rcS");
    fs::write(&rcs, generate_rc_local()).with_context(|| format!("writing {}", rcs.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&rcs, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod {}", rcs.display()))?;
    }
    Ok(())
}

/// Resolve the workspace root from `xtask`'s own `CARGO_MANIFEST_DIR`.
fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("resolving workspace root from CARGO_MANIFEST_DIR")
        .map(Path::to_path_buf)
}

/// Cross-compile `miniboxd` for [`AGENT_TARGET_TRIPLE`] and stage it into
/// `rootfs/sbin/minibox-agent`, alongside the busybox-init files that start
/// it with `MINIBOX_VSOCK_PORT` set. Runs on every `build-vm-image`
/// invocation (cargo's own incremental build makes repeats cheap) so the
/// rootfs stays correct even when the ESP itself is cache-hit and skipped.
fn provision_rootfs(vm_dir: &Path, workspace_root: &Path) -> Result<()> {
    let rootfs_dir = vm_dir.join("rootfs");
    fs::create_dir_all(&rootfs_dir)
        .with_context(|| format!("creating {}", rootfs_dir.display()))?;

    println!("  cargo build --release --target {AGENT_TARGET_TRIPLE} -p miniboxd");
    let status = Command::new("cargo")
        .current_dir(workspace_root)
        .args([
            "build",
            "--release",
            "--target",
            AGENT_TARGET_TRIPLE,
            "-p",
            "miniboxd",
        ])
        .status()
        .context("spawning cargo build for the guest agent")?;
    if !status.success() {
        bail!("cargo build --release --target {AGENT_TARGET_TRIPLE} -p miniboxd failed");
    }

    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map_or_else(|_| workspace_root.join("target"), PathBuf::from);
    let built_binary = target_dir
        .join(AGENT_TARGET_TRIPLE)
        .join("release")
        .join("miniboxd");
    anyhow::ensure!(
        built_binary.exists(),
        "expected built agent binary at {} but it is missing",
        built_binary.display()
    );

    let sbin = rootfs_dir.join("sbin");
    fs::create_dir_all(&sbin).with_context(|| format!("creating {}", sbin.display()))?;
    let agent_path = sbin.join("minibox-agent");
    fs::copy(&built_binary, &agent_path).with_context(|| {
        format!(
            "copying {} to {}",
            built_binary.display(),
            agent_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&agent_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod {}", agent_path.display()))?;
    }

    install_init_files(&rootfs_dir)?;
    println!("  staged  {}", agent_path.display());
    println!(
        "  wrote   {}/etc/inittab, {}/etc/init.d/rcS",
        rootfs_dir.display(),
        rootfs_dir.display()
    );

    install_kernel_modules(
        vm_dir,
        &rootfs_dir,
        VSOCK_MODULES,
        "vsock-module-extract",
        "vsock",
    )?;
    install_kernel_modules(
        vm_dir,
        &rootfs_dir,
        NET_MODULES,
        "net-module-extract",
        "net",
    )?;
    install_kernel_modules(vm_dir, &rootfs_dir, FS_MODULES, "fs-module-extract", "fs")?;
    Ok(())
}

/// Stage the given `.ko` files onto the rootfs itself (not just the initramfs).
///
/// `GUEST_MODULES`/`modules=` on the kernel cmdline asks the Alpine initramfs's own
/// init to `modprobe` these before it mounts the virtiofs root, but that mechanism
/// was observed not to actually load them (`socket(AF_VSOCK)` still failed with
/// `EAFNOSUPPORT` after `switch_root`, and the same was later observed for `AF_PACKET`).
/// `rcS` now `insmod`s these explicitly, in dependency order, as a belt-and-suspenders fix.
fn install_kernel_modules(
    vm_dir: &Path,
    rootfs_dir: &Path,
    specs: &[ModuleSpec],
    staging_name: &str,
    label: &str,
) -> Result<()> {
    let cache = vm_dir.join("cache");
    fs::create_dir_all(&cache).with_context(|| format!("creating {}", cache.display()))?;
    let apk = cache.join(LINUX_VIRT_PACKAGE);
    ensure_download(LINUX_VIRT_URL, &apk, LINUX_VIRT_SHA256)?;

    let staging = cache.join(staging_name);
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let modules = read_modules(&apk, &staging, specs)?;

    for (spec, data) in &modules {
        let rel_dir = Path::new(spec.path)
            .parent()
            .with_context(|| format!("module path {} has no parent dir", spec.path))?;
        let module_dir = rootfs_dir
            .join("lib/modules")
            .join(KERNEL_RELEASE)
            .join(rel_dir);
        fs::create_dir_all(&module_dir)
            .with_context(|| format!("creating {}", module_dir.display()))?;
        let file_name = Path::new(spec.path)
            .file_name()
            .with_context(|| format!("module path {} has no file name", spec.path))?;
        let dest = module_dir.join(file_name);
        fs::write(&dest, data).with_context(|| format!("writing {}", dest.display()))?;
        println!("  staged  {label} module into {}", dest.display());
    }
    Ok(())
}

/// Build the ESP and boot assets, skipping the work when nothing changed.
pub fn run(options: &Options) -> Result<()> {
    let vm_dir = &options.vm_dir;
    provision_rootfs(vm_dir, &workspace_root()?)?;

    let cache = vm_dir.join("cache");
    let efi_dir = vm_dir.join("efi");
    let esp_path = efi_dir.join(vm_boot::ESP_IMAGE);
    let manifest_path = efi_dir.join(MANIFEST_FILE);
    fs::create_dir_all(&efi_dir).with_context(|| format!("creating {}", efi_dir.display()))?;

    let cmdline = vm_boot::kernel_cmdline();
    if !options.force
        && let Some(manifest) = read_matching_manifest(&manifest_path, &esp_path, &cmdline)?
    {
        println!(
            "  cached  {} (manifest matches pinned inputs; use --force to rebuild)",
            esp_path.display()
        );
        println!("  esp sha256 {}", manifest.esp_sha256);
        return Ok(());
    }

    println!("build-vm-image: {ALPINE_RELEASE} {ALPINE_ARCH}");
    let iso = cache.join(ISO_FILE);
    ensure_download(ISO_URL, &iso, ISO_SHA256)?;
    let apk = cache.join(LINUX_VIRT_PACKAGE);
    ensure_download(LINUX_VIRT_URL, &apk, LINUX_VIRT_SHA256)?;

    let staging = cache.join("extract");
    if staging.exists() {
        fs::remove_dir_all(&staging).with_context(|| format!("clearing {}", staging.display()))?;
    }
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;

    let kernel_source = staging.join("vmlinuz-virt");
    extract_member(&iso, "boot/vmlinuz-virt", &kernel_source, false)?;
    let initramfs_source = staging.join("initramfs-virt");
    extract_member(&iso, "boot/initramfs-virt", &initramfs_source, false)?;
    let grub_source = staging.join("bootaa64.efi");
    extract_member(&iso, "efi/boot/bootaa64.efi", &grub_source, false)?;
    println!("  extract boot/vmlinuz-virt, boot/initramfs-virt, efi/boot/bootaa64.efi");

    let all_module_specs: Vec<ModuleSpec> = VSOCK_MODULES
        .iter()
        .chain(NET_MODULES)
        .chain(FS_MODULES)
        .copied()
        .collect();
    let modules = read_modules(&apk, &staging, &all_module_specs)?;
    let alpine_cpio = gzip_decode(&initramfs_source)?;
    let initramfs = augment_initramfs(&alpine_cpio, &modules)?;

    let kernel = fs::read(&kernel_source).context("reading extracted kernel")?;
    let grub = fs::read(&grub_source).context("reading extracted GRUB binary")?;
    let files = vec![
        EspFilePlan {
            path: vm_boot::ESP_BOOT_BINARY.to_string(),
            data: grub,
        },
        EspFilePlan {
            path: vm_boot::ESP_GRUB_CFG.to_string(),
            data: render_grub_cfg(&cmdline).into_bytes(),
        },
        EspFilePlan {
            path: vm_boot::ESP_KERNEL.to_string(),
            data: kernel,
        },
        EspFilePlan {
            path: vm_boot::ESP_INITRAMFS.to_string(),
            data: initramfs,
        },
    ];

    let mut esp_files = build_fat_image(&esp_path, ESP_SIZE_BYTES, &files)?;
    esp_files.sort_by(|a, b| a.path.cmp(&b.path));
    let esp_sha256 = sha256_file(&esp_path)?;
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        alpine_release: ALPINE_RELEASE.to_string(),
        iso_sha256: ISO_SHA256.to_string(),
        linux_virt_sha256: LINUX_VIRT_SHA256.to_string(),
        kernel_cmdline: cmdline,
        esp_files,
        esp_sha256,
    };
    let json = serde_json::to_string_pretty(&manifest).context("serializing manifest")?;
    fs::write(&manifest_path, format!("{json}\n"))
        .with_context(|| format!("writing {}", manifest_path.display()))?;

    println!("  wrote   {}", esp_path.display());
    for file in &manifest.esp_files {
        println!(
            "    {:<28} {:>9} bytes  {}",
            file.path, file.size, file.sha256
        );
    }
    println!("  esp sha256 {}", manifest.esp_sha256);
    println!("  manifest {}", manifest_path.display());
    Ok(())
}

fn read_matching_manifest(
    manifest_path: &Path,
    esp_path: &Path,
    cmdline: &str,
) -> Result<Option<Manifest>> {
    if !manifest_path.exists() || !esp_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: Manifest = match serde_json::from_str(&raw) {
        Ok(manifest) => manifest,
        Err(error) => {
            println!("  rebuild: unreadable manifest ({error})");
            return Ok(None);
        }
    };
    let inputs_match = manifest.format == MANIFEST_FORMAT
        && manifest.iso_sha256 == ISO_SHA256
        && manifest.linux_virt_sha256 == LINUX_VIRT_SHA256
        && manifest.kernel_cmdline == cmdline;
    if !inputs_match {
        println!("  rebuild: manifest does not match pinned inputs");
        return Ok(None);
    }
    let actual = sha256_file(esp_path)?;
    if actual != manifest.esp_sha256 {
        println!(
            "  rebuild: {} does not match its manifest hash",
            esp_path.display()
        );
        return Ok(None);
    }
    Ok(Some(manifest))
}

/// Unused helper kept for parity with the old QEMU image builder: extract an ISO member.
#[allow(dead_code)]
fn iso_member(iso: &Path, member: &str) -> Result<Vec<u8>> {
    let output = Command::new("tar")
        .args(["-xOf"])
        .arg(iso)
        .arg(member)
        .output()
        .with_context(|| format!("running tar for {member}"))?;
    if !output.status.success() {
        bail!(
            "tar failed to read {member} from {}: {}",
            iso.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn module(spec: &ModuleSpec, bytes: &[u8]) -> (ModuleSpec, Vec<u8>) {
        (*spec, bytes.to_vec())
    }

    fn alpine_initramfs_cpio() -> Vec<u8> {
        let dep = "kernel/fs/fuse/fuse.ko:\nkernel/fs/fuse/virtiofs.ko: kernel/fs/fuse/fuse.ko\n"
            .to_string();
        write_newc(&[
            CpioEntry {
                name: "init".to_string(),
                mode: MODE_FILE,
                data: b"#!/bin/sh\n".to_vec(),
            },
            CpioEntry {
                name: "lib/modules/6.12.114-0-virt/modules.dep".to_string(),
                mode: MODE_FILE,
                data: dep.into_bytes(),
            },
        ])
    }

    #[test]
    fn newc_round_trips_entries_and_concatenated_archives() {
        let first = write_newc(&[CpioEntry {
            name: "a/b".to_string(),
            mode: MODE_FILE,
            data: b"hello".to_vec(),
        }]);
        let second = write_newc(&[CpioEntry {
            name: "c".to_string(),
            mode: MODE_DIR,
            data: Vec::new(),
        }]);
        let mut combined = first;
        combined.extend_from_slice(&second);
        let entries = parse_newc(&combined).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a/b", "c"]
        );
        assert_eq!(entries[0].data, b"hello");
    }

    #[test]
    fn augment_adds_modules_and_extends_modules_dep() {
        let modules = vec![
            module(&VSOCK_MODULES[0], b"vsock"),
            module(&VSOCK_MODULES[1], b"common"),
            module(&VSOCK_MODULES[2], b"transport"),
        ];
        let gz = augment_initramfs(&alpine_initramfs_cpio(), &modules).unwrap();
        let cpio = {
            let mut decoder = GzDecoder::new(Cursor::new(gz));
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).unwrap();
            out
        };
        let entries = parse_newc(&cpio).unwrap();
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"lib/modules/6.12.114-0-virt/kernel/net/vmw_vsock/vsock.ko"));
        assert!(names.contains(
            &"lib/modules/6.12.114-0-virt/kernel/net/vmw_vsock/vmw_vsock_virtio_transport.ko"
        ));
        assert!(names.contains(&"lib/modules/6.12.114-0-virt/kernel/net/vmw_vsock"));

        let dep = entries
            .iter()
            .rfind(|entry| entry.name == "lib/modules/6.12.114-0-virt/modules.dep")
            .unwrap();
        let dep = String::from_utf8(dep.data.clone()).unwrap();
        assert!(dep.contains("kernel/fs/fuse/virtiofs.ko"), "{dep}");
        assert!(
            dep.contains("kernel/net/vmw_vsock/vmw_vsock_virtio_transport.ko: kernel/net/vmw_vsock/vsock.ko kernel/net/vmw_vsock/vmw_vsock_virtio_transport_common.ko"),
            "{dep}"
        );
    }

    #[test]
    fn augment_skips_modules_already_present() {
        let mut cpio = alpine_initramfs_cpio();
        let entries = parse_newc(&cpio).unwrap();
        let mut extended = entries;
        extended.push(CpioEntry {
            name: format!("lib/modules/{KERNEL_RELEASE}/{}", VSOCK_MODULES[0].path),
            mode: MODE_FILE,
            data: b"already".to_vec(),
        });
        cpio = write_newc(&extended);

        let modules = vec![module(&VSOCK_MODULES[0], b"replacement")];
        let gz = augment_initramfs(&cpio, &modules).unwrap();
        let mut decoder = GzDecoder::new(Cursor::new(gz));
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        let entries = parse_newc(&out).unwrap();
        let vsock: Vec<_> = entries
            .iter()
            .filter(|entry| entry.name.ends_with("vsock.ko"))
            .collect();
        assert_eq!(vsock.len(), 1);
        assert_eq!(vsock[0].data, b"already");
    }

    #[test]
    fn augment_rejects_missing_modules_dep() {
        let cpio = write_newc(&[CpioEntry {
            name: "init".to_string(),
            mode: MODE_FILE,
            data: b"#!/bin/sh\n".to_vec(),
        }]);
        let error = augment_initramfs(&cpio, &[module(&VSOCK_MODULES[0], b"vsock")]).unwrap_err();
        assert!(error.to_string().contains("modules.dep"), "{error}");
    }

    #[test]
    fn grub_cfg_boots_the_kernel_and_initramfs_with_the_shared_cmdline() {
        let cmdline = vm_boot::kernel_cmdline();
        let cfg = render_grub_cfg(&cmdline);
        assert!(cfg.contains("linux /boot/vmlinuz-virt"), "{cfg}");
        assert!(cfg.contains("initrd /boot/initramfs-virt"), "{cfg}");
        assert!(cfg.contains(&cmdline), "{cfg}");
        assert!(cfg.contains("set timeout=0"), "{cfg}");
    }

    #[test]
    fn fat_image_is_deterministic_and_contains_every_file() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first.img");
        let second = dir.path().join("second.img");
        let files = vec![
            EspFilePlan {
                path: "EFI/BOOT/BOOTAA64.EFI".to_string(),
                data: b"grub".to_vec(),
            },
            EspFilePlan {
                path: "boot/grub/grub.cfg".to_string(),
                data: b"cfg".to_vec(),
            },
        ];
        let manifest = build_fat_image(&first, 4 * 1024 * 1024, &files).unwrap();
        build_fat_image(&second, 4 * 1024 * 1024, &files).unwrap();
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        assert_eq!(manifest.len(), 2);
        assert_eq!(manifest[0].sha256, sha256_hex(b"grub"));
    }

    #[test]
    fn fat_image_round_trips_file_contents() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("esp.img");
        let files = vec![EspFilePlan {
            path: "boot/grub/grub.cfg".to_string(),
            data: b"menuentry".to_vec(),
        }];
        build_fat_image(&path, 4 * 1024 * 1024, &files).unwrap();
        let file = File::open(&path).unwrap();
        let fs = FileSystem::new(file, FsOptions::new()).unwrap();
        let mut read = String::new();
        fs.root_dir()
            .open_dir("boot")
            .unwrap()
            .open_dir("grub")
            .unwrap()
            .open_file("grub.cfg")
            .unwrap()
            .read_to_string(&mut read)
            .unwrap();
        assert_eq!(read, "menuentry");
    }

    #[test]
    fn manifest_skip_requires_matching_cmdline_and_hash() {
        let dir = tempdir().unwrap();
        let esp = dir.path().join("boot.img");
        let manifest_path = dir.path().join(MANIFEST_FILE);
        fs::write(&esp, b"esp").unwrap();
        let manifest = Manifest {
            format: MANIFEST_FORMAT,
            alpine_release: ALPINE_RELEASE.to_string(),
            iso_sha256: ISO_SHA256.to_string(),
            linux_virt_sha256: LINUX_VIRT_SHA256.to_string(),
            kernel_cmdline: "cmdline".to_string(),
            esp_files: Vec::new(),
            esp_sha256: sha256_hex(b"esp"),
        };
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        assert!(
            read_matching_manifest(&manifest_path, &esp, "cmdline")
                .unwrap()
                .is_some()
        );
        assert!(
            read_matching_manifest(&manifest_path, &esp, "other")
                .unwrap()
                .is_none()
        );
        fs::write(&esp, b"tampered").unwrap();
        assert!(
            read_matching_manifest(&manifest_path, &esp, "cmdline")
                .unwrap()
                .is_none()
        );
    }
}
