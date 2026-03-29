//! VZVirtualMachine wrapper — implementation in Task 7.

/// Configuration for booting the minibox Linux VM.
#[derive(Debug, Clone)]
pub struct VzVmConfig {
    pub vm_dir: std::path::PathBuf,
    pub images_dir: std::path::PathBuf,
    pub containers_dir: std::path::PathBuf,
    pub memory_bytes: u64,
    pub cpu_count: usize,
}

impl VzVmConfig {
    pub fn kernel_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("boot").join("vmlinuz-virt")
    }
    pub fn initramfs_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("boot").join("initramfs-virt")
    }
    pub fn rootfs_path(&self) -> std::path::PathBuf {
        self.vm_dir.join("rootfs")
    }
}

/// Handle to a running Virtualization.framework Linux VM — stub for Task 7.
pub struct VzVm;

impl VzVm {
    pub fn stop(&self) {
        // stub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vz_vm_config_fields_are_accessible() {
        let tmp = std::env::temp_dir();
        let cfg = VzVmConfig {
            vm_dir: tmp.clone(),
            images_dir: tmp.clone(),
            containers_dir: tmp.clone(),
            memory_bytes: 512 * 1024 * 1024,
            cpu_count: 2,
        };
        assert_eq!(cfg.cpu_count, 2);
    }

    #[test]
    fn vz_vm_config_paths_use_vm_dir() {
        let vm_dir = std::path::PathBuf::from("/tmp/test-vm");
        let cfg = VzVmConfig {
            vm_dir: vm_dir.clone(),
            images_dir: Default::default(),
            containers_dir: Default::default(),
            memory_bytes: 0,
            cpu_count: 0,
        };
        assert_eq!(cfg.kernel_path(), vm_dir.join("boot").join("vmlinuz-virt"));
        assert_eq!(cfg.rootfs_path(), vm_dir.join("rootfs"));
    }
}
