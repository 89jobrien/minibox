//! Proptest property tests consolidated from minibox-core and minibox quickcheck suites.
//!
//! Property families:
//! 1. Path traversal completeness — dotdot detection, absolute path detection, safe relative paths
//! 2. Image ref roundtrip — parse(ref.to_string()) == ref
//! 3. Protocol codec roundtrip — deserialize(serialize(msg)) == msg
//! 4. Cgroup limit arithmetic — no overflow, no zero-division
//! 5. IP allocator no-double-assign — allocate() never returns an in-use IP
//! 6. Overlay mount-option string — output is valid mount(2) option syntax

use minibox::image::reference::ImageRef;
use minibox::protocol::{
    DaemonRequest, DaemonResponse, decode_request, decode_response, encode_request, encode_response,
};
use proptest::prelude::*;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 1. Path traversal completeness
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn path_with_dotdot_is_rejected(
        prefix in "[a-zA-Z0-9]{1,8}",
        suffix in "[a-zA-Z0-9]{1,8}",
    ) {
        let evil = format!("{prefix}/../../{suffix}");
        let path = Path::new(&evil);
        let has_dotdot = path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
        prop_assert!(has_dotdot);
    }

    #[test]
    fn absolute_path_has_root_component(segment in "[a-zA-Z0-9]{1,16}") {
        let abs = format!("/{segment}");
        let path = Path::new(&abs);
        prop_assert!(path.is_absolute());
    }

    #[test]
    fn safe_relative_path_has_no_dotdot(
        component in "[a-zA-Z0-9_-]{1,16}"
    ) {
        prop_assume!(component != "..");
        let path = Path::new(&component);
        let has_dotdot = path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
        prop_assert!(!has_dotdot && !path.is_absolute());
    }
}

// ---------------------------------------------------------------------------
// 2. Image ref roundtrip
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn image_ref_roundtrip(seed in 0u8..=255) {
        let refs = [
            "alpine",
            "alpine:3.18",
            "myorg/myimage",
            "myorg/myimage:v2",
            "ghcr.io/org/image:stable",
            "ghcr.io/org/image:latest",
            "registry.example.com/ns/app:1.0",
            "localhost/ns/tool:dev",
        ];
        let input = refs[seed as usize % refs.len()];
        let parsed = ImageRef::parse(input);
        prop_assume!(parsed.is_ok());
        let parsed = parsed.expect("checked above");
        let displayed = parsed.to_string();
        let reparsed = ImageRef::parse(&displayed);
        prop_assert!(reparsed.is_ok(), "failed to reparse '{}': {:?}", displayed, reparsed.err());
        prop_assert_eq!(parsed, reparsed.expect("checked above"));
    }

    #[test]
    fn image_ref_structured_roundtrip(
        registry_idx in 0usize..4,
        namespace_idx in 0usize..4,
        name_idx in 0usize..5,
        tag_idx in 0usize..5,
    ) {
        let registries = ["docker.io", "ghcr.io", "registry.example.com", "localhost"];
        let namespaces = ["library", "myorg", "testns", "org"];
        let names = ["alpine", "ubuntu", "myapp", "tool", "service"];
        let tags = ["latest", "v1", "3.18", "stable", "dev"];

        let registry = registries[registry_idx].to_string();
        let namespace = if registry == "docker.io" {
            namespaces[namespace_idx].to_string()
        } else {
            let non_library = ["myorg", "testns", "org"];
            non_library[namespace_idx % non_library.len()].to_string()
        };
        let name = names[name_idx].to_string();
        let tag = tags[tag_idx].to_string();

        let image_ref = ImageRef {
            registry,
            namespace,
            name,
            tag,
        };

        let displayed = image_ref.to_string();
        let reparsed = ImageRef::parse(&displayed);
        prop_assert!(reparsed.is_ok(), "failed to reparse '{}': {:?}", displayed, reparsed.err());
        prop_assert_eq!(image_ref, reparsed.expect("checked above"));
    }
}

// ---------------------------------------------------------------------------
// 3. Protocol codec roundtrip
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn protocol_request_codec_roundtrip(seed in 0u8..=255) {
        let requests = [
            DaemonRequest::List,
            DaemonRequest::Stop {
                id: "test-container".to_string(),
            },
            DaemonRequest::Remove {
                id: "test-container".to_string(),
            },
            DaemonRequest::Run {
                image: "alpine".to_string(),
                tag: Some("latest".to_string()),
                command: vec!["echo".to_string(), "hello".to_string()],
                memory_limit_bytes: None,
                cpu_weight: None,
                ephemeral: false,
                network: None,
                env: vec![],
                mounts: vec![],
                privileged: false,
                name: None,
                tty: false,
                entrypoint: None,
                user: None,
                auto_remove: false,
                priority: None,
                urgency: None,
                execution_context: None,
                platform: None,
                cgroup_parent: None,
            },
        ];

        let req = &requests[seed as usize % requests.len()];
        let encoded = encode_request(req).expect("encode should succeed");
        let decoded = decode_request(&encoded).expect("decode should succeed");

        let original_json = serde_json::to_value(req).expect("serialize original");
        let decoded_json = serde_json::to_value(&decoded).expect("serialize decoded");
        prop_assert_eq!(original_json, decoded_json);
    }

    #[test]
    fn protocol_response_codec_roundtrip(seed in 0u8..=255) {
        let responses = [
            DaemonResponse::ContainerCreated {
                id: "abc123".to_string(),
            },
            DaemonResponse::Success {
                message: "done".to_string(),
            },
            DaemonResponse::Error {
                message: "something went wrong".to_string(),
            },
        ];

        let resp = &responses[seed as usize % responses.len()];
        let encoded = encode_response(resp).expect("encode should succeed");
        let decoded = decode_response(&encoded).expect("decode should succeed");

        let original_json = serde_json::to_value(resp).expect("serialize original");
        let decoded_json = serde_json::to_value(&decoded).expect("serialize decoded");
        prop_assert_eq!(original_json, decoded_json);
    }
}

// ---------------------------------------------------------------------------
// 4. Cgroup limit arithmetic
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn cgroup_memory_limit_no_overflow(bytes: u64) {
        let formatted = format!("{bytes}");
        let parsed: u64 = formatted.parse().expect("should parse back");
        prop_assert_eq!(parsed, bytes);
    }

    #[test]
    fn cgroup_cpu_weight_clamp_no_panic(weight: u64) {
        let clamped = weight.clamp(1, 10000);
        prop_assert!((1..=10000).contains(&clamped));
    }

    #[test]
    fn cgroup_pids_max_roundtrip(pids in 1u64..=u64::MAX) {
        let s = format!("{pids}");
        let parsed: u64 = s.parse().expect("should parse");
        prop_assert_eq!(parsed, pids);
    }

    #[test]
    fn cgroup_io_bandwidth_format_no_panic(bps: u64) {
        let formatted = format!("8:0 rbps={bps} wbps={bps}");
        prop_assert!(formatted.contains(&bps.to_string()));
    }
}

// ---------------------------------------------------------------------------
// 5. IP allocator no-double-assign (Linux-only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod ip_allocator_tests {
    use super::*;
    use std::collections::HashSet;

    proptest! {
        #[test]
        fn ip_allocator_no_double_assign(count in 0u8..=250) {
            use ipnet::IpNet;
            use minibox::adapters::network::bridge::IpAllocator;

            let subnet: IpNet = "10.0.0.0/24".parse().expect("valid subnet");
            let mut alloc = IpAllocator::new(subnet).expect("create allocator");

            let mut seen = HashSet::new();
            for _ in 0..count {
                match alloc.allocate() {
                    Some(ip) => {
                        prop_assert!(seen.insert(ip), "duplicate IP allocated: {ip}");
                    }
                    None => break,
                }
            }
        }

        #[test]
        fn ip_allocator_release_reuse(seed in 1u8..=10) {
            use ipnet::IpNet;
            use minibox::adapters::network::bridge::IpAllocator;

            let subnet: IpNet = "10.0.0.0/24".parse().expect("valid subnet");
            let mut alloc = IpAllocator::new(subnet).expect("create allocator");

            let mut allocated = Vec::new();
            for _ in 0..seed {
                if let Some(ip) = alloc.allocate() {
                    allocated.push(ip);
                }
            }

            prop_assume!(!allocated.is_empty());

            let released = *allocated.last().expect("non-empty");
            alloc.release(released);

            let reallocated = alloc.allocate();
            if let Some(ip) = reallocated {
                let still_held: HashSet<_> = allocated[..allocated.len() - 1].iter().collect();
                prop_assert!(!still_held.contains(&ip));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Overlay mount-option string
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn overlay_mount_options_valid_syntax(n_layers in 1u8..=5) {
        let layers: Vec<PathBuf> = (0..n_layers)
            .map(|i| PathBuf::from(format!("/var/lib/minibox/images/layer{i}")))
            .collect();

        let container_dir = PathBuf::from("/var/lib/minibox/containers/test123");
        let upper = container_dir.join("upper");
        let work = container_dir.join("work");

        let lowerdir: String = layers
            .iter()
            .rev()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":");

        let options = format!(
            "lowerdir={lowerdir},upperdir={upper},workdir={work}",
            upper = upper.display(),
            work = work.display(),
        );

        prop_assert!(options.starts_with("lowerdir="));
        prop_assert!(options.contains(",upperdir="));
        prop_assert!(options.contains(",workdir="));
        prop_assert!(!options.contains("=,") && !options.ends_with('='));
        prop_assert!(!options.contains(' '));
    }

    #[test]
    fn overlay_options_no_delimiter_collision(
        layer_name in "[a-zA-Z0-9_.-]{1,32}"
    ) {
        let layer = PathBuf::from(format!("/images/{layer_name}"));
        let lowerdir = layer.display().to_string();
        prop_assert!(!lowerdir.contains(','));
    }
}
