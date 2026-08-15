#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools,
    clippy::duration_suboptimal_units,
    clippy::incompatible_msrv,
    clippy::suspicious_map,
    clippy::unnecessary_map_or,
    clippy::let_unit_value,
    clippy::ignored_unit_patterns
)]

//! Deterministic synthetic OCI layer builders for extraction/pull benches.

// Bench fixture code: panicking on a broken fixture is the correct behaviour.

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

/// Shape of a synthetic OCI layer for extraction/pull benches.
#[derive(Debug, Clone, Copy)]
pub struct LayerSpec {
    pub file_count: usize,
    pub file_size_bytes: usize,
    pub dir_depth: usize,
}

/// Deterministic gzipped tar built from the spec.
///
/// Content is fully deterministic: fixed payload byte, zero mtimes, and
/// paths derived only from the spec — identical specs produce identical
/// bytes (and therefore identical digests).
#[must_use]
pub fn build_layer_tar_gz(spec: &LayerSpec) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    let payload = vec![0xA5u8; spec.file_size_bytes];
    for i in 0..spec.file_count {
        let dir: String = (0..spec.dir_depth).map(|d| format!("d{d}/")).collect();
        let path = format!("{dir}file-{i:06}.bin");
        let mut header = tar::Header::new_gnu();
        header.set_size(spec.file_size_bytes as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, payload.as_slice())
            .expect("append tar entry");
    }
    builder
        .into_inner()
        .expect("finish tar")
        .finish()
        .expect("finish gzip")
}

/// OCI-style digest string (`sha256:<hex>`) for a byte slice.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;

    fn small_spec() -> LayerSpec {
        LayerSpec {
            file_count: 3,
            file_size_bytes: 1024,
            dir_depth: 2,
        }
    }

    #[test]
    fn build_is_deterministic() {
        let a = build_layer_tar_gz(&small_spec());
        let b = build_layer_tar_gz(&small_spec());
        assert_eq!(a, b, "identical specs must produce identical bytes");
    }

    #[test]
    fn built_layer_gunzips_with_expected_entry_count() {
        let bytes = build_layer_tar_gz(&small_spec());
        let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
        let count = archive
            .entries()
            .expect("tar entries")
            .map(|e| e.expect("valid tar entry"))
            .count();
        assert_eq!(count, 3, "archive must contain exactly file_count entries");
    }

    #[test]
    fn extract_layer_round_trips_through_real_consumer() {
        let spec = small_spec();
        let bytes = build_layer_tar_gz(&spec);
        let dest = tempfile::TempDir::new().expect("tempdir");

        let mut reader = bytes.as_slice();
        minibox_core::image::layer::extract_layer(&mut reader, dest.path())
            .expect("extract_layer must accept fixture layers");

        for i in 0..spec.file_count {
            let file = dest.path().join(format!("d0/d1/file-{i:06}.bin"));
            let meta = std::fs::metadata(&file).expect("extracted file exists");
            assert_eq!(meta.len(), spec.file_size_bytes as u64);
        }
    }

    #[test]
    fn sha256_digest_has_oci_format() {
        let digest = sha256_digest(b"abc");
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), "sha256:".len() + 64);
    }
}
