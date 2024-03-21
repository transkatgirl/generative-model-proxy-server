use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use http::uri::{Authority, Parts, PathAndQuery, Scheme, Uri};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime, trace, Resource};
use reqwest::{Client, ClientBuilder};
use sled::{Db, Mode}; // sled should probably be replaced with a proper database at some point. will need to write manual migrations when that time comes.
use tokio::{fs, net::TcpListener, signal};
use tracing::Level;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod limiter;
mod model;

use limiter::LimiterClock;

/// A multi-user proxy server for major generative model APIs
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// The internet socket address that the HTTP server will be available on.
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    bind_to: SocketAddr,

    /// The location of the folder used to store the proxy's database.
    #[arg(short, long, default_value = "./database")]
    database_folder: PathBuf,

    /// The OpenTelemetry-compatible collector used for logging.
    /// Signals sent to the collector may contain sensitive information.
    #[arg(short, long)]
    opentelemetry_endpoint: Option<String>,
}

#[derive(Clone)]
struct AppState {
    http: Client,
    database: Db,
    clock: Arc<LimiterClock>,
}

// TODO: Implement a system for handling database migrations
//const PAST_DATABASE_STRING: &str = "version-0";
//const FUTURE_DATABASE_STRING: &str = "version-2"; // ? How will we handle version rollbacks?
const CURRENT_DATABASE_STRING: &str = "version-1";

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let registry = tracing_subscriber::registry()
        .with(
            filter::Targets::new()
                .with_default(Level::TRACE)
                .with_targets(vec![
                    ("rustls", Level::INFO),
                    ("trust_dns_proto", Level::INFO),
                    ("trust_dns_resolver", Level::INFO),
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
                        .with_endpoint(endpoint.clone()),
                )
                .with_trace_config(trace::config().with_resource(Resource::new(vec![
                    KeyValue::new("service.name", "generative-model-proxy-server"),
                ])))
                .install_batch(runtime::Tokio)
                .context("Failed to start OpenTelemetry tracing pipeline")?;
            let meter = opentelemetry_otlp::new_pipeline()
                .metrics(runtime::Tokio)
                .with_exporter(
                    opentelemetry_otlp::new_exporter()
                        .tonic()
                        .with_endpoint(endpoint),
                )
                .with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    "generative-model-proxy-server",
                )]))
                .build()
                .context("Failed to start OpenTelemetry metrics pipeline")?;
            let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
            let metrics = MetricsLayer::new(meter);

            registry.with(metrics).with(telemetry).init()
        }
        None => registry.init(),
    }

    fs::create_dir_all(&args.database_folder)
        .await
        .context("Unable to create database directory!")?;

    let database_location = args
        .database_folder
        .join(PathBuf::from(CURRENT_DATABASE_STRING));

    let state = AppState {
        http: ClientBuilder::new()
            .user_agent("generative-model-proxy-server")
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Some(Duration::from_secs(5)))
            .http2_keep_alive_timeout(Duration::from_secs(15))
            .http2_keep_alive_while_idle(true)
            .build()
            .context("Unable to initalize HTTP client")?,
        database: sled::Config::default()
            .path(&database_location)
            .mode(Mode::HighThroughput)
            .open()
            .context("Unable to initalize database")?,
        clock: Arc::new(LimiterClock::new()),
    };

    let listener = TcpListener::bind(&args.bind_to)
        .await
        .with_context(|| format!("Failed to bind HTTP server to {}", &args.bind_to))?;

    if state.is_table_empty("users") {
        let addr = listener.local_addr().unwrap_or(args.bind_to);
        let mut parts = Parts::default();
        parts.scheme = Some(Scheme::HTTP);
        parts.authority = Authority::try_from(format!("{}", addr)).ok();
        parts.path_and_query = Some(PathAndQuery::from_static("/admin/help"));

        let uri = match Uri::from_parts(parts) {
            Ok(uri) => format!("{}", uri),
            Err(_) => "/admin/help".to_string(),
        };

        tracing::warn!("It looks like you don't have any users added to your database. Please see {} (login with a blank username and \"setup-key\" as the password) for more information.", uri)
    }

    axum::serve(listener, api::api_router(state.clone()))
        .with_graceful_shutdown(async move {
            if let Err(error) = signal::ctrl_c().await {
                tracing::error!("Unable to run signal handler task: {}", error)
            }
        })
        .await
        .context("Failed to start HTTP server")?;

    tracing::debug!("flushing database to disk");
    if let Err(error) = state.database.flush_async().await {
        tracing::error!("Unable to flush database to disk: {}", error)
    }

    Ok(())
}
