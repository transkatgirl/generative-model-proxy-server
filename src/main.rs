use clap::Parser;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace,
    Resource,
};
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod limiter;
mod model;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    bind_to: String,

    #[arg(short, long)]
    opentelemetry_endpoint: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let registry = tracing_subscriber::registry()
        .with(
            filter::Targets::new()
                .with_default(Level::TRACE)
                .with_targets(vec![
                    ("h2", Level::INFO),
                    ("hyper", Level::INFO),
                    ("tower", Level::INFO),
                    ("tokio_util", Level::INFO),
                    ("tonic", Level::INFO),
                    ("tower_http", Level::DEBUG),
                ]),
        )
        .with(tracing_subscriber::fmt::layer().pretty());

    match args.opentelemetry_endpoint {
        Some(endpoint) => {
            let tracer = opentelemetry_otlp::new_pipeline()
                .tracing()
                .with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .tonic()
                        .with_endpoint(endpoint),
                )
                .with_trace_config(trace::config().with_resource(Resource::new(vec![
                    KeyValue::new("service.name", "language-model-proxy-server"),
                ])))
                .install_batch(opentelemetry_sdk::runtime::Tokio)
                .unwrap();
            let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

            registry.with(telemetry).init()
        }
        None => registry.init(),
    }

    let listener = TcpListener::bind(args.bind_to).await.unwrap();
    axum::serve(listener, api::api_router().await)
        .await
        .unwrap();
}
