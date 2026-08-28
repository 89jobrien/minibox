//! Capability protocol compatibility tests.
#![allow(clippy::expect_used, clippy::panic, reason = "test assertions")]
use minibox_core::domain::{Backend, Capability, CapabilitySupport, capability_matrix};
use minibox_core::protocol::{
    DaemonRequest, DaemonResponse, decode_request, decode_response, encode_request, encode_response,
};

#[test]
fn capability_request_roundtrips_without_changing_existing_wire_tags() {
    let encoded = encode_request(&DaemonRequest::GetCapabilities).expect("encode request");
    assert_eq!(encoded, b"{\"type\":\"GetCapabilities\"}\n");
    assert!(matches!(
        decode_request(&encoded).expect("decode request"),
        DaemonRequest::GetCapabilities
    ));
}

#[test]
fn capability_response_roundtrips_typed_matrix() {
    let encoded = encode_response(&DaemonResponse::CapabilityMatrix {
        matrix: capability_matrix(),
    })
    .expect("encode response");
    let decoded = decode_response(&encoded).expect("decode response");
    match decoded {
        DaemonResponse::CapabilityMatrix { matrix } => {
            assert_eq!(
                matrix.support(Backend::Colima, Capability::Exec),
                Some(CapabilitySupport::Limited)
            );
        }
        other => panic!("expected capability matrix, got {other:?}"),
    }
}
