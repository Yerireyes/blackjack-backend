//! API HTTP opcional sobre el motor: análisis remoto (mismo contrato que WASM),
//! cuentas, sesiones de juego y estadísticas. Prefijo `/api/v1`.

pub mod auth;
pub mod client_ip;
pub mod config;
pub mod error;
pub mod extract;
pub mod metrics;
pub mod names;
pub mod rooms;
pub mod routes;
pub mod state;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Router;
use governor::middleware::NoOpMiddleware;
use tower_governor::governor::{GovernorConfig, GovernorConfigBuilder};
use tower_governor::{GovernorError, GovernorLayer};
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::auth::JwtKeys;
use crate::client_ip::{ClientIpKeyExtractor, UserKeyExtractor};
use crate::config::Config;
use crate::error::ApiError;
use crate::state::AppState;

pub type Limiter = Arc<GovernorConfig<ClientIpKeyExtractor, NoOpMiddleware>>;
pub type UserLimiter = Arc<GovernorConfig<UserKeyExtractor, NoOpMiddleware>>;

/// Límites de tasa por IP del cliente (ver [`client_ip`]): análisis y rutas de cuenta
/// (registro, login, Google, nombre). Las acciones de juego van por usuario.
pub struct RateLimits {
    pub analyze: Limiter,
    pub auth: Limiter,
    pub play: UserLimiter,
}

impl RateLimits {
    pub fn new(cfg: &Config) -> RateLimits {
        let key = ClientIpKeyExtractor::new(&cfg.trusted_proxies);
        // Juego: 5 por segundo con ráfaga configurable (10 por defecto) por usuario.
        let play = GovernorConfigBuilder::default()
            .key_extractor(UserKeyExtractor::new(
                key.clone(),
                JwtKeys::new(&cfg.jwt_secret),
            ))
            .per_millisecond(200)
            .burst_size(cfg.play_burst.max(1))
            .finish()
            .expect("configuración de límite de tasa válida");
        let analyze = GovernorConfigBuilder::default()
            .key_extractor(key.clone())
            .per_millisecond(cfg.analyze_replenish_ms.max(1))
            .burst_size(cfg.analyze_burst.max(1))
            .finish()
            .expect("configuración de límite de tasa válida");
        // Cuentas: ráfaga configurable (10 por defecto) y luego una cada 6 s por IP.
        let auth = GovernorConfigBuilder::default()
            .key_extractor(key)
            .per_second(6)
            .burst_size(cfg.auth_burst.max(1))
            .finish()
            .expect("configuración de límite de tasa válida");
        RateLimits {
            analyze: Arc::new(analyze),
            auth: Arc::new(auth),
            play: Arc::new(play),
        }
    }
}

fn rate_limited(err: GovernorError) -> Response<Body> {
    match err {
        GovernorError::TooManyRequests { wait_time, .. } => {
            // governor trunca a segundos enteros: con recargas de menos de 1 s saldría 0.
            let wait_time = wait_time.max(1);
            let mut res = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                format!("demasiadas peticiones; reintenta en {wait_time} s"),
            )
            .into_response();
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait_time));
            res
        }
        GovernorError::UnableToExtractKey => {
            ApiError::internal("no se pudo obtener la IP del cliente").into_response()
        }
        GovernorError::Other { code, msg, .. } => {
            ApiError::new(code, "rate_limited", msg.unwrap_or_default()).into_response()
        }
    }
}

async fn not_found() -> ApiError {
    ApiError::not_found("ruta no encontrada")
}

/// El front compilado de `STATIC_DIR`, si tiene `index.html`: front y API en el mismo
/// origen (cookie `SameSite=Lax`, WebSocket y login con Google sin CORS). Las rutas que no
/// son archivos devuelven `index.html` (la SPA resuelve la pantalla).
fn web(cfg: &Config) -> Option<Router> {
    let dir = cfg.static_dir.as_ref()?;
    let index = dir.join("index.html");
    if !index.is_file() {
        tracing::warn!(
            "STATIC_DIR={} no tiene index.html: no se sirve el front",
            dir.display()
        );
        return None;
    }
    let files = ServeDir::new(dir).fallback(ServeFile::new(index));
    Some(
        Router::new()
            .fallback_service(files)
            .layer(axum::middleware::map_response(web_cache)),
    )
}

/// Vite pone un hash en el nombre de todo lo de `/assets/`: se cachea un año. El resto
/// (`index.html` sobre todo) se revalida siempre, para que un despliegue se vea enseguida.
async fn web_cache(uri: axum::http::Uri, mut res: Response) -> Response {
    let value = if uri.path().starts_with("/assets/") && res.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    res
}

/// Construye la aplicación. Necesita `ConnectInfo<SocketAddr>` (límite por IP).
pub fn app(state: AppState, cfg: &Config, limits: &RateLimits) -> Router {
    use routes::{
        achievements, analyze, auth, casual, cosmetics, daily, friends, google, moderation, play,
        ranked, sessions, stats,
    };

    let analyze_routes = Router::new()
        .route("/analyze/hand", post(analyze::hand))
        .route("/analyze/dealer", post(analyze::dealer))
        .route("/analyze/round", post(analyze::round))
        .layer(GovernorLayer::new(limits.analyze.clone()).error_handler(rate_limited));

    let auth_routes = Router::new()
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/google/start", get(google::start))
        .route("/auth/google/callback", get(google::callback))
        .route("/auth/google/link-intent", post(google::link_intent))
        .route("/auth/google/unlink", post(google::unlink))
        .route("/me", patch(auth::update_me))
        .layer(GovernorLayer::new(limits.auth.clone()).error_handler(rate_limited));

    let play_routes = Router::new()
        .route("/casual/runs", post(casual::create_run))
        .route("/casual/runs/{id}/retire", post(casual::retire))
        .route("/daily/attempts", post(daily::start))
        .route("/rooms", post(rooms::create))
        .route("/rooms/{code}/invite", post(friends::invite))
        .route("/friends/requests", post(friends::request))
        .route("/reports", post(moderation::report))
        .route("/friends/requests/{id}/accept", post(friends::accept))
        .route("/friends/requests/{id}/decline", post(friends::decline))
        .route("/friends/{id}/remove", post(friends::remove))
        .route("/play/rounds", post(play::deal))
        .route("/play/rounds/{id}/actions", post(play::act))
        .route("/play/rounds/{id}/insurance", post(play::insurance))
        .layer(GovernorLayer::new(limits.play.clone()).error_handler(rate_limited));

    let other_routes = Router::new()
        .route("/casual/runs/active", get(casual::active_run))
        .route(
            "/casual/leaderboard/weekly",
            get(casual::weekly_leaderboard),
        )
        .route("/me/records", get(casual::records))
        .route("/me/achievements", get(achievements::mine))
        .route("/me/history", get(stats::history))
        .route("/friends", get(friends::list))
        .route("/admin/reports", get(moderation::reports))
        .route("/admin/reports/{id}/dismiss", post(moderation::dismiss))
        .route("/admin/users", get(moderation::users))
        .route("/admin/users/{id}/rename", post(moderation::rename))
        .route("/admin/users/{id}/ban", post(moderation::ban))
        .route("/admin/users/{id}/unban", post(moderation::unban))
        .route("/me/invitations/{id}/dismiss", post(friends::dismiss))
        .route("/me/stats", get(stats::stats))
        .route("/cosmetics", get(cosmetics::catalog))
        .route("/rooms", get(rooms::list))
        .route("/rooms/{code}", get(rooms::get))
        .route("/rooms/{code}/ws", get(rooms::ws))
        .route("/me/cosmetics", get(cosmetics::mine).put(cosmetics::equip))
        .route("/ranked/me", get(ranked::me))
        .route("/ranked/leaderboard", get(ranked::leaderboard))
        .route("/daily/me", get(daily::me))
        .route("/daily/leaderboard", get(daily::leaderboard))
        .route("/play/active", get(play::active))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/logout-all", post(auth::logout_all))
        .route("/auth/me", get(auth::me))
        .route("/auth/providers", get(auth::providers))
        .route("/sessions", post(sessions::create).get(sessions::list))
        .route("/sessions/{id}/hands", post(sessions::add_hand))
        .route("/sessions/{id}/stats", get(sessions::stats))
        .route("/sessions/{id}/end", post(sessions::end))
        .route("/health", get(routes::health))
        .route("/metrics", get(metrics::serve));

    let api = analyze_routes
        .merge(auth_routes)
        .merge(play_routes)
        .merge(other_routes)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            metrics::count,
        ))
        .with_state(state);

    let origin = cfg
        .frontend_origin
        .parse::<HeaderValue>()
        .expect("FRONTEND_ORIGIN debe ser un origen válido");
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::PUT])
        .allow_headers([header::CONTENT_TYPE]);

    // Dentro de /api/v1 una ruta desconocida es un 404 JSON, también cuando el resto lo
    // sirve el front.
    let api = api.fallback(not_found);
    let router = Router::new().nest("/api/v1", api);
    let router = match web(cfg) {
        Some(web) => router.fallback_service(web),
        None => router.fallback(not_found),
    };
    router
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
