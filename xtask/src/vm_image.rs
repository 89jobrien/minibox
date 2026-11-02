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
pub const KERNEL_RELEASE: &str = "6.12.114-0-virt";

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

fn read_modules(apk: &Path, staging: &Path) -> Result<Vec<(ModuleSpec, Vec<u8>)>> {
    let mut modules = Vec::with_capacity(VSOCK_MODULES.len());
    for spec in VSOCK_MODULES {
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

/// Build the ESP and boot assets, skipping the work when nothing changed.
pub fn run(options: &Options) -> Result<()> {
    let vm_dir = &options.vm_dir;
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

    let modules = read_modules(&apk, &staging)?;
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
