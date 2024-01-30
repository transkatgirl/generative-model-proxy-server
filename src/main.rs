use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

mod api;
mod model;
mod limiter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
    axum::serve(
        listener,
        api::api_router().layer(TraceLayer::new_for_http()),
    )
    .await
    .unwrap();
}
