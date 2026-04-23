//! Conformance tests for the `AgentError` conversion contract.
//!
//! These tests verify the externally-visible guarantees of `AgentError`:
//! - Every variant must be constructible from its source error type.
//! - `Display` must be non-empty for every variant.
//! - `std::error::Error::source` must return `Some` for every variant.
//! - The `Debug` output must contain the inner variant name.
//!
//! Add a new test block whenever a new `AgentError` variant is introduced.

use minibox_agent::AgentError;
use minibox_llm::LlmError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn llm_err(msg: &str) -> LlmError {
    LlmError::AllProvidersFailed(msg.to_string())
}

fn crux_err(step: &str, msg: &str) -> cruxai_core::types::error::CruxErr {
    cruxai_core::types::error::CruxErr::step_failed(step, msg)
}

// ---------------------------------------------------------------------------
// AgentError::Llm conformance
// ---------------------------------------------------------------------------

#[test]
fn llm_variant_display_is_non_empty() {
    let err = AgentError::Llm(llm_err("test"));
    assert!(!err.to_string().is_empty(), "Display must not be empty");
}

#[test]
fn llm_variant_source_is_some() {
    let err = AgentError::Llm(llm_err("source check"));
    assert!(
        std::error::Error::source(&err).is_some(),
        "source must be Some for Llm variant"
    );
}

#[test]
fn llm_variant_source_contains_original_message() {
    let err = AgentError::Llm(llm_err("original-sentinel-1234"));
    let source = std::error::Error::source(&err).unwrap();
    assert!(
        source.to_string().contains("original-sentinel-1234"),
        "source should contain original message, got: {source}"
    );
}

#[test]
fn llm_variant_debug_contains_inner_variant() {
    let err = AgentError::Llm(llm_err("debug-check"));
    let debug = format!("{err:?}");
    assert!(
        debug.contains("AllProvidersFailed"),
        "Debug must show inner LlmError variant, got: {debug}"
    );
}

#[test]
fn llm_variant_from_conversion_works() {
    let source = llm_err("from-test");
    let err: AgentError = AgentError::from(source);
    assert!(matches!(err, AgentError::Llm(_)));
}

// ---------------------------------------------------------------------------
// AgentError::Step conformance
// ---------------------------------------------------------------------------

#[test]
fn step_variant_display_is_non_empty() {
    let err = AgentError::Step(crux_err("step_name", "some failure"));
    assert!(!err.to_string().is_empty(), "Display must not be empty");
}

#[test]
fn step_variant_source_is_some() {
    let err = AgentError::Step(crux_err("step_name", "source check"));
    assert!(
        std::error::Error::source(&err).is_some(),
        "source must be Some for Step variant"
    );
}

#[test]
fn step_variant_source_contains_original_message() {
    let err = AgentError::Step(crux_err("step_name", "sentinel-step-5678"));
    let source = std::error::Error::source(&err).unwrap();
    assert!(
        source.to_string().contains("sentinel-step-5678"),
        "source should contain original message, got: {source}"
    );
}

#[test]
fn step_variant_from_conversion_works() {
    let source = crux_err("s", "msg");
    let err: AgentError = AgentError::from(source);
    assert!(matches!(err, AgentError::Step(_)));
}

// ---------------------------------------------------------------------------
// Exhaustiveness guard — fails to compile if new variants are added
// ---------------------------------------------------------------------------

#[test]
fn all_variants_are_covered() {
    let err: AgentError = AgentError::Llm(llm_err("x"));
    // This match must be exhaustive. If a new variant is added without a
    // conformance block above, the compiler will catch it here.
    match err {
        AgentError::Llm(_) => {}
        AgentError::Step(_) => {}
    }
}
