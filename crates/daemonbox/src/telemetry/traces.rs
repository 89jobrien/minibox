//! OTEL trace exporter setup.
//!
//! Replaces the bare `tracing_subscriber::fmt().init()` in main.rs with a
//! layered subscriber that optionally adds OTLP trace export.
//!
//! Uses opentelemetry 0.31 APIs:
//! - `SdkTracerProvider` (not the removed `TracerProvider`)
//! - `.with_batch_exporter()` without runtime param (SDK manages its own threads since 0.28)
//! - `provider.shutdown()` on the instance (not the removed `global::shutdown_tracer_provider()`)

use opentelemetry::trace::TracerProvider as _;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Initialize the tracing subscriber with optional OTLP trace export.
///
/// - `otlp_endpoint = None` → fmt-only logging (existing behavior)
/// - `otlp_endpoint = Some(url)` → fmt + OTEL trace export to the given endpoint
///
/// Returns an [`OtelGuard`] that must be held for the lifetime of the program.
/// On drop, it flushes pending spans via `SdkTracerProvider::shutdown()`.
pub fn init_tracing(otlp_endpoint: Option<&str>) -> OtelGuard {
    // Collect all layers into a Vec<Box<dyn Layer<Registry>>> so the registry
    // only gets a single `.with(layers)` call, keeping `S = Registry` throughout.
    // This avoids the type mismatch that occurs when stacking Optional<OtelLayer>
    // on top of an already-layered Layered<EnvFilter, Registry>.
    let mut layers: Vec<Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>> = vec![
        tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("miniboxd=info".parse().unwrap())
            .boxed(),
        tracing_subscriber::fmt::layer().boxed(),
    ];

    let provider = if let Some(endpoint) = otlp_endpoint {
        match build_otel_layer(endpoint) {
            Ok((otel_layer, prov)) => {
                layers.push(otel_layer.boxed());
                Some(prov)
            }
            Err(e) => {
                // Fall back to fmt-only if OTEL init fails.
                eprintln!("[minibox] OTEL trace init failed, falling back to fmt-only: {e}");
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry().with(layers).init();

    OtelGuard { provider }
}

fn build_otel_layer(
    endpoint: &str,
) -> Result<
    (
        tracing_opentelemetry::OpenTelemetryLayer<
            tracing_subscriber::Registry,
            opentelemetry_sdk::trace::Tracer,
        >,
        opentelemetry_sdk::trace::SdkTracerProvider,
    ),
    Box<dyn std::error::Error>,
> {
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    // SDK manages batch export threads internally since 0.28 — no runtime param needed.
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("miniboxd");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Ok((layer, provider))
}

/// Guard that shuts down the OTEL tracer provider on drop.
///
/// Hold this in `main()`. If OTLP was not configured, drop is a no-op.
///
/// Note: `global::shutdown_tracer_provider()` was removed in opentelemetry 0.28.
/// Must call `.shutdown()` on the `SdkTracerProvider` instance directly.
pub struct OtelGuard {
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take()
            && let Err(e) = provider.shutdown()
        {
            eprintln!("[minibox] OTEL tracer shutdown error: {e}");
        }
    }
}
