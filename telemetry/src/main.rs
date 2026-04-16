mod db;
mod handlers;
mod models;
mod telegram;
mod utils;

use axum::{routing::post, Router};
use db::init_db;
use handlers::{handle_auto_join, handle_ping, handle_start_auto, handle_start_manual};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .init();

    let pool = init_db().await;

    let app = Router::new()
        .route("/voxelproxy/ping", post(handle_ping))
        .route("/voxelproxy/v1/ping", post(handle_ping))
        .route("/voxelproxy/v1/start_manual", post(handle_start_manual))
        .route("/voxelproxy/v1/start_auto", post(handle_start_auto))
        .route("/voxelproxy/v1/auto_join", post(handle_auto_join))
        .layer(TraceLayer::new_for_http())
        .with_state(pool);

    let addr: SocketAddr = "127.0.0.1:1111".parse().unwrap();
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    info!("Server running on {}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
