use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

mod api;
mod limiter;
mod model;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
    axum::serve(listener, api::api_router().await)
        .await
        .unwrap();
}
