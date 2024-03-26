use std::env;
use std::error::Error;
use std::time::Duration;

use axum::body::{Body, BoxBody};
use http::{Request, Response};
use opentelemetry_otlp::WithExportConfig;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_tracing(
    service_name: Option<&str>,
    otlp_endpoint: Option<&str>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint(
            otlp_endpoint.unwrap_or(opentelemetry_otlp::OTEL_EXPORTER_OTLP_ENDPOINT_DEFAULT),
        ))
        .with_trace_config(
            opentelemetry_sdk::trace::config()
                .with_resource(opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new(
                        opentelemetry_semantic_conventions::resource::SERVICE_NAME,
                        service_name.unwrap_or(env!("CARGO_PKG_NAME")).to_string(),
                    ),
                    opentelemetry::KeyValue::new(
                        opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
                        env!("CARGO_PKG_VERSION"),
                    ),
                ]))
                .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                    opentelemetry_sdk::trace::Sampler::AlwaysOn,
                ))),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    tracing_subscriber::registry()
        .with(
            tracing_opentelemetry::layer()
                .with_error_records_to_exceptions(true)
                .with_tracer(tracer),
        )
        .with(EnvFilter::builder().parse("info,otel::tracing=trace,otel=debug")?)
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_timer(tracing_subscriber::fmt::time::time()),
        )
        .init();

    Ok(())
}
// Custom function for creating request-level spans
// tracing crate requires all fields to be defined at creation time, so any fields that will be set
// later should be defined as Empty
pub fn make_span(request: &Request<Body>) -> Span {
    use opentelemetry::trace::TraceContextExt;

    let span = tracing::info_span!(
        "request",
        method = %request.method(),
        uri = %request.uri(),
        version = ?request.version(),
        status = tracing::field::Empty,
        latency = tracing::field::Empty,
    );

    // Get parent trace id from headers, if available
    // This uses OTel extension set_parent rather than setting field directly on the span to ensure
    // it works no matter which propagator is configured
    let parent_context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&opentelemetry_http::HeaderExtractor(request.headers()))
    });
    // if there is no parent span ID, we get something nonsensical, so we need to validate it
    // (yes, this is hilarious)
    let parent_context_span = parent_context.span();
    let parent_context_span_context = parent_context_span.span_context();
    if parent_context_span_context.is_valid() {
        span.set_parent(parent_context);
    }

    span
}

// Custom function for adding information to request-level span that is only available at response time.
pub fn on_response(response: &Response<BoxBody>, latency: Duration, span: &Span) {
    span.record("status", tracing::field::display(response.status()));
    span.record("latency", tracing::field::display(latency.as_nanos()));
}