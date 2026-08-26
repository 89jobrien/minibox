//! Cross-platform validation of cgroup v2 resource limit bounds.
//!
//! Pure and filesystem-independent so it builds and runs on any platform,
//! unlike `container::cgroups::CgroupManager::create` (Linux-only, requires
//! a real cgroup v2 mount) which calls [`validate_resource_limits`] before
//! writing any limit files.

/// Minimum memory limit in bytes (kernel minimum is typically 4KB).
pub const MIN_MEMORY_BYTES: u64 = 4096;

/// Maximum cgroup v2 CPU weight (kernel range is 1-10000).
pub const MAX_CPU_WEIGHT: u64 = 10000;

/// Validate resource limits against kernel-imposed bounds.
///
/// # Security
///
/// Rejects a memory limit below the kernel minimum and a CPU weight outside
/// the kernel-accepted range, preventing a caller from requesting
/// effectively-unlimited or invalid resource constraints.
pub fn validate_resource_limits(
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
) -> anyhow::Result<()> {
    if let Some(mem) = memory_limit_bytes
        && mem < MIN_MEMORY_BYTES
    {
        anyhow::bail!("memory limit must be >= {MIN_MEMORY_BYTES} bytes, got {mem}");
    }
    if let Some(cpu) = cpu_weight
        && !(1..=MAX_CPU_WEIGHT).contains(&cpu)
    {
        anyhow::bail!("cpu_weight must be 1-{MAX_CPU_WEIGHT}, got {cpu}");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn accepts_no_limits() {
        assert!(validate_resource_limits(None, None).is_ok());
    }

    #[test]
    fn rejects_memory_below_minimum() {
        assert!(validate_resource_limits(Some(100), None).is_err());
    }

    #[test]
    fn accepts_memory_at_minimum() {
        assert!(validate_resource_limits(Some(4096), None).is_ok());
    }

    #[test]
    fn rejects_zero_cpu_weight() {
        assert!(validate_resource_limits(None, Some(0)).is_err());
    }

    #[test]
    fn rejects_cpu_weight_above_max() {
        assert!(validate_resource_limits(None, Some(10_001)).is_err());
    }

    #[test]
    fn accepts_cpu_weight_in_range() {
        assert!(validate_resource_limits(None, Some(100)).is_ok());
    }

    #[test]
    fn accepts_cpu_weight_at_max() {
        assert!(validate_resource_limits(None, Some(10_000)).is_ok());
    }
}
