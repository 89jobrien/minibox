//! Minimal protective-MBR + GPT writer for the VZ guest's EFI system partition.
//!
//! Operates entirely on an in-memory byte buffer, not a `File` — `pwrite`/`write_at` into a
//! pre-zeroed file produced a disk image `VZDiskImageStorageDeviceAttachment` rejected
//! (`VZErrorDomain code=2: storage device attachment is invalid`) even once GPT-partitioned
//! and fully materialized, while a plain `dd` byte-copy of the exact same content always
//! passed. The suspected cause is the positioned-write (scattered `pwrite`) pattern itself
//! interacting with APFS extent allocation in a way `stat`/`du` don't surface — not file
//! content, size, path, or xattrs (all ruled out by direct A/B testing). Building the whole
//! image in memory and writing it out with one sequential `write_all` call sidesteps the
//! question entirely by matching `dd`'s write pattern. See
//! `.ctx/godmode/BLOCKED-vz-pivot-root-einval.md` ("NEW BLOCKER (2026-11-07)").

use anyhow::{Context, Result};

const SECTOR_SIZE: u64 = 512;
/// 1 MiB alignment for the partition start, matching common UEFI firmware images.
const PARTITION_START_LBA: u64 = 2048;
/// Sectors reserved at the end of the disk for the backup partition entry array (32) and
/// backup GPT header (1).
const BACKUP_RESERVED_SECTORS: u64 = 33;
const PARTITION_ENTRY_COUNT: u32 = 128;
const PARTITION_ENTRY_SIZE: u32 = 128;
const PARTITION_ENTRY_ARRAY_SECTORS: u64 =
    (PARTITION_ENTRY_COUNT as u64 * PARTITION_ENTRY_SIZE as u64).div_ceil(SECTOR_SIZE);

/// EFI System Partition type GUID, as defined by the UEFI spec.
const ESP_TYPE_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";

/// Byte range of the EFI System Partition within the disk image.
#[derive(Debug, Clone, Copy)]
pub struct PartitionWindow {
    pub offset: u64,
    pub len: u64,
}

/// Parse a hyphenated GUID string into its on-disk mixed-endian byte representation.
fn guid_bytes(guid: &str) -> [u8; 16] {
    let hex_only: String = guid.chars().filter(|c| *c != '-').collect();
    let mut raw = [0u8; 16];
    for (index, byte) in raw.iter_mut().enumerate() {
        let start = index * 2;
        *byte =
            u8::from_str_radix(&hex_only[start..start + 2], 16).expect("GUID literal is valid hex");
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&raw[0..4]);
    out[0..4].reverse();
    out[4..6].copy_from_slice(&raw[4..6]);
    out[4..6].reverse();
    out[6..8].copy_from_slice(&raw[6..8]);
    out[6..8].reverse();
    out[8..16].copy_from_slice(&raw[8..16]);
    out
}

/// Deterministic, fixed GUID derived from a seed byte so repeated builds are reproducible.
fn fixed_guid(seed: u8) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = seed.wrapping_add(index as u8);
    }
    bytes
}

fn utf16_name(name: &str, field_len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(field_len);
    for unit in name.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.resize(field_len, 0);
    out
}

fn partition_entry(
    type_guid: [u8; 16],
    unique_guid: [u8; 16],
    start_lba: u64,
    end_lba: u64,
) -> Vec<u8> {
    let mut entry = vec![0u8; PARTITION_ENTRY_SIZE as usize];
    entry[0..16].copy_from_slice(&type_guid);
    entry[16..32].copy_from_slice(&unique_guid);
    entry[32..40].copy_from_slice(&start_lba.to_le_bytes());
    entry[40..48].copy_from_slice(&end_lba.to_le_bytes());
    // attributes (8 bytes) stay zero
    let name = utf16_name("EFI System Partition", 72);
    entry[56..128].copy_from_slice(&name);
    entry
}

fn entry_array(entry: &[u8]) -> Vec<u8> {
    let mut array = vec![0u8; (PARTITION_ENTRY_COUNT * PARTITION_ENTRY_SIZE) as usize];
    array[..entry.len()].copy_from_slice(entry);
    array
}

#[allow(clippy::too_many_arguments)]
fn gpt_header(
    my_lba: u64,
    alternate_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    disk_guid: [u8; 16],
    partition_entry_lba: u64,
    entry_array_crc32: u32,
) -> [u8; SECTOR_SIZE as usize] {
    let mut header = [0u8; SECTOR_SIZE as usize];
    header[0..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    header[12..16].copy_from_slice(&92u32.to_le_bytes());
    // header[16..20] CRC32 filled in below, zeroed for its own computation
    header[24..32].copy_from_slice(&my_lba.to_le_bytes());
    header[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
    header[40..48].copy_from_slice(&first_usable_lba.to_le_bytes());
    header[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
    header[56..72].copy_from_slice(&disk_guid);
    header[72..80].copy_from_slice(&partition_entry_lba.to_le_bytes());
    header[80..84].copy_from_slice(&PARTITION_ENTRY_COUNT.to_le_bytes());
    header[84..88].copy_from_slice(&PARTITION_ENTRY_SIZE.to_le_bytes());
    header[88..92].copy_from_slice(&entry_array_crc32.to_le_bytes());

    let crc = crc32fast::hash(&header[0..92]);
    header[16..20].copy_from_slice(&crc.to_le_bytes());
    header
}

fn protective_mbr(total_sectors: u64) -> [u8; SECTOR_SIZE as usize] {
    let mut mbr = [0u8; SECTOR_SIZE as usize];
    let size_in_lba = u32::try_from(total_sectors.saturating_sub(1)).unwrap_or(u32::MAX);
    let entry = &mut mbr[446..462];
    entry[0] = 0x00; // not bootable
    entry[1..4].copy_from_slice(&[0x00, 0x02, 0x00]); // starting CHS (unused by GPT-aware tools)
    entry[4] = 0xEE; // GPT protective partition type
    entry[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]); // ending CHS (unused)
    entry[8..12].copy_from_slice(&1u32.to_le_bytes()); // starting LBA
    entry[12..16].copy_from_slice(&size_in_lba.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    mbr
}

fn write_into(buffer: &mut [u8], offset: u64, data: &[u8]) -> Result<()> {
    let start = usize::try_from(offset).context("offset too large for in-memory buffer")?;
    let end = start + data.len();
    anyhow::ensure!(
        end <= buffer.len(),
        "write of {} bytes at offset {offset} overruns the {}-byte buffer",
        data.len(),
        buffer.len()
    );
    buffer[start..end].copy_from_slice(data);
    Ok(())
}

/// Compute the byte range the single EFI System Partition will occupy for a disk of
/// `total_bytes`, without writing anything. Pure and deterministic, so callers (including
/// tests) can locate the partition after [`write_protective_gpt`] has run.
pub fn partition_window(total_bytes: u64) -> Result<PartitionWindow> {
    anyhow::ensure!(
        total_bytes.is_multiple_of(SECTOR_SIZE),
        "disk size {total_bytes} is not a multiple of the sector size"
    );
    let total_sectors = total_bytes / SECTOR_SIZE;
    anyhow::ensure!(
        total_sectors > PARTITION_START_LBA + BACKUP_RESERVED_SECTORS,
        "disk of {total_sectors} sectors is too small for a GPT + ESP layout"
    );

    let first_usable_lba = 2 + PARTITION_ENTRY_ARRAY_SECTORS;
    let last_usable_lba = total_sectors - 1 - BACKUP_RESERVED_SECTORS;
    let partition_start_lba = PARTITION_START_LBA.max(first_usable_lba);
    let partition_end_lba = last_usable_lba;
    anyhow::ensure!(
        partition_end_lba > partition_start_lba,
        "no room left for the ESP after reserving GPT structures"
    );

    Ok(PartitionWindow {
        offset: partition_start_lba * SECTOR_SIZE,
        len: (partition_end_lba - partition_start_lba + 1) * SECTOR_SIZE,
    })
}

/// Write a protective MBR + GPT (primary and backup) into `buffer`, which must be exactly
/// `total_bytes` long. Returns the byte range of the single EFI System Partition.
pub fn write_protective_gpt(buffer: &mut [u8], total_bytes: u64) -> Result<PartitionWindow> {
    anyhow::ensure!(
        buffer.len() as u64 == total_bytes,
        "buffer is {} bytes, expected {total_bytes}",
        buffer.len()
    );
    let window = partition_window(total_bytes)?;
    let total_sectors = total_bytes / SECTOR_SIZE;
    let first_usable_lba = 2 + PARTITION_ENTRY_ARRAY_SECTORS;
    let last_usable_lba = total_sectors - 1 - BACKUP_RESERVED_SECTORS;
    let partition_start_lba = window.offset / SECTOR_SIZE;
    let partition_end_lba = partition_start_lba + window.len / SECTOR_SIZE - 1;

    let disk_guid = fixed_guid(0x4d);
    let partition_guid = fixed_guid(0x45);
    let type_guid = guid_bytes(ESP_TYPE_GUID);
    let entry = partition_entry(
        type_guid,
        partition_guid,
        partition_start_lba,
        partition_end_lba,
    );
    let array = entry_array(&entry);
    let array_crc32 = crc32fast::hash(&array);

    let primary_entry_lba = 2u64;
    let backup_entry_lba = total_sectors - 1 - BACKUP_RESERVED_SECTORS + 1;

    let primary_header = gpt_header(
        1,
        total_sectors - 1,
        first_usable_lba,
        last_usable_lba,
        disk_guid,
        primary_entry_lba,
        array_crc32,
    );
    let backup_header = gpt_header(
        total_sectors - 1,
        1,
        first_usable_lba,
        last_usable_lba,
        disk_guid,
        backup_entry_lba,
        array_crc32,
    );

    write_into(buffer, 0, &protective_mbr(total_sectors))?;
    write_into(buffer, SECTOR_SIZE, &primary_header)?;
    write_into(buffer, primary_entry_lba * SECTOR_SIZE, &array)?;
    write_into(buffer, backup_entry_lba * SECTOR_SIZE, &array)?;
    write_into(buffer, (total_sectors - 1) * SECTOR_SIZE, &backup_header)?;

    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_bytes_matches_known_mixed_endian_encoding() {
        // EFI System Partition GUID's first dword, byte-swapped: C12A7328 -> 28 73 2A C1.
        let bytes = guid_bytes(ESP_TYPE_GUID);
        assert_eq!(&bytes[0..4], &[0x28, 0x73, 0x2A, 0xC1]);
    }

    #[test]
    fn write_protective_gpt_produces_a_valid_signature_and_partition_window() {
        let size = 8 * 1024 * 1024u64;
        let mut buffer = vec![0u8; size as usize];

        let window = write_protective_gpt(&mut buffer, size).expect("write gpt");
        assert!(window.offset >= 1024 * 1024);
        assert!(window.offset + window.len <= size);

        assert_eq!(&buffer[510..512], &[0x55, 0xAA]);
        assert_eq!(buffer[446 + 4], 0xEE);
        assert_eq!(&buffer[512..520], b"EFI PART");
    }
}
