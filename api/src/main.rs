use std::net::SocketAddr;
use std::time::Duration;

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

    let play = limits.play.limiter().clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(60));
        loop {
            every.tick().await;
            play.retain_recent();
        }
    });

    // Borra cada hora las sesiones de login vencidas o revocadas.
    let purge_pool = pool.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(3600));
        loop {
            every.tick().await;
            match api::auth::purge_sessions(&purge_pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("sesiones vencidas borradas: {n}"),
                Err(e) => tracing::warn!("no se pudieron borrar las sesiones vencidas: {e}"),
            }
        }
    });

    // Cierra cada hora las rondas abandonadas y las temporadas ranked terminadas.
    let game_pool = pool.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(3600));
        loop {
            every.tick().await;
            api::routes::play::housekeeping(&game_pool).await;
        }
    });

    if cfg.google.is_none() {
        tracing::info!(
            "login con Google deshabilitado (faltan GOOGLE_CLIENT_ID y GOOGLE_CLIENT_SECRET)"
        );
    }
    let state = AppState::new(pool, &cfg);
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
