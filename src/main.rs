use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace, Resource};
use reqwest::{Client, ClientBuilder};
use sled::Db; // sled should probably be replaced with a proper database at some point. will need to write manual migrations when that time comes.
use tokio::net::TcpListener;
use tracing::Level;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod limiter;
mod model;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    bind_to: String,

    #[arg(short, long, default_value = "database.sled")]
    database_file: String,

    #[arg(short, long)]
    opentelemetry_endpoint: Option<String>,
}

#[derive(Clone)]
struct AppState {
    http: Client,
    database: Db,
}

#[tokio::main]
async fn main() -> Result<()> {
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
                    ("sled", Level::INFO),
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
                .context("Failed to start OpenTelemetry")?;
            let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

            registry.with(telemetry).init()
        }
        None => registry.init(),
    }

    let state = AppState {
        http: ClientBuilder::new()
            .user_agent("language-model-proxy-server")
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Some(Duration::from_secs(5)))
            .http2_keep_alive_timeout(Duration::from_secs(15))
            .http2_keep_alive_while_idle(true)
            .build()
            .context("Unable to initalize HTTP client")?,
        database: sled::open(&args.database_file).context("Unable to initalize database")?,
    };

    let listener = TcpListener::bind(&args.bind_to)
        .await
        .with_context(|| format!("Failed to bind HTTP server to {}", &args.bind_to))?;
    axum::serve(listener, api::api_router(state).await)
        .await
        .context("Failed to start HTTP server")?;

    // TODO: Graceful shutdown

    Ok(())
}
