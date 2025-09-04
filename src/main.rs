use std::net::SocketAddr;

use actix_files::{Files, NamedFile};
use actix_web::{App, HttpServer, Scope};
use game_server::init_game_server;
use tracing::info;
use tracing_actix_web::TracingLogger;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod game_server;

#[actix::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    HttpServer::new(move || {
        let app = App::new().wrap(TracingLogger::default());
        if cfg!(feature = "client") {
            app.service(Scope::new("/api").configure(init_game_server))
                .service(Files::new("/", "client/build").index_file("index.html"))
                .default_service(
                    NamedFile::open("client/build/dynamic.html")
                        .expect("couldn't open fallback file; was the frontend built?"),
                )
        } else {
            app.configure(init_game_server)
        }
    })
    .bind(SocketAddr::from(([127, 0, 0, 1], 3001)))
    .inspect(|server| {
        server
            .addrs()
            .iter()
            .for_each(|addr| info!("listening on http://{addr}"))
    })?
    .run()
    .await
}
