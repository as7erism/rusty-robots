use axum::Router;
use game_server::init_game_server;
use std::net::SocketAddr;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod game_server;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app = if cfg!(feature = "client") {
        Router::new()
            .fallback_service(
                ServeDir::new("client/build").fallback(ServeFile::new("client/build/dynamic.html")),
            )
            .nest("/api", init_game_server())
    } else {
        init_game_server()
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 3003));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::debug!("listening on http://{}", listener.local_addr().unwrap());
    axum::serve(listener, app.layer(TraceLayer::new_for_http()))
        .with_graceful_shutdown((|| async {
            tokio::signal::ctrl_c().await.unwrap();
            println!("handling ctrlc");
        })())
        .await
        .unwrap();
}
