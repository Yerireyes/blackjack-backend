use std::net::SocketAddr;
use std::time::Duration;

use api::auth::JwtKeys;
use api::config::Config;
use api::state::AppState;
use api::{app, RateLimits};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "api=info,tower_http=info".into()),
        )
        .init();

    let cfg = Config::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&cfg.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let limits = RateLimits::new(&cfg);
    // Limpia periódicamente las IPs inactivas de los limitadores.
    for limiter in [
        limits.analyze.limiter().clone(),
        limits.auth.limiter().clone(),
    ] {
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(60));
            loop {
                every.tick().await;
                limiter.retain_recent();
            }
        });
    }

    let state = AppState {
        pool,
        jwt: JwtKeys::new(&cfg.jwt_secret),
        cookie_secure: cfg.cookie_secure,
    };
    let router = app(state, &cfg, &limits);
    let listener = tokio::net::TcpListener::bind(cfg.bind_addr).await?;
    tracing::info!("API escuchando en http://{}/api/v1", cfg.bind_addr);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
    })
    .await?;
    Ok(())
}
