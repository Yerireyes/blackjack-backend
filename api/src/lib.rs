//! API HTTP opcional sobre el motor: análisis remoto (mismo contrato que WASM),
//! cuentas, sesiones de juego y estadísticas. Prefijo `/api/v1`.

pub mod auth;
pub mod client_ip;
pub mod config;
pub mod error;
pub mod extract;
pub mod names;
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
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::client_ip::ClientIpKeyExtractor;
use crate::config::Config;
use crate::error::ApiError;
use crate::state::AppState;

pub type Limiter = Arc<GovernorConfig<ClientIpKeyExtractor, NoOpMiddleware>>;

/// Límites de tasa por IP del cliente (ver [`client_ip`]): análisis y rutas de cuenta
/// (registro, login, Google, nombre).
pub struct RateLimits {
    pub analyze: Limiter,
    pub auth: Limiter,
}

impl RateLimits {
    pub fn new(cfg: &Config) -> RateLimits {
        let key = ClientIpKeyExtractor::new(&cfg.trusted_proxies);
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
        }
    }
}

fn rate_limited(err: GovernorError) -> Response<Body> {
    match err {
        GovernorError::TooManyRequests { wait_time, .. } => {
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

/// Construye la aplicación. Necesita `ConnectInfo<SocketAddr>` (límite por IP).
pub fn app(state: AppState, cfg: &Config, limits: &RateLimits) -> Router {
    use routes::{analyze, auth, google, sessions};

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

    let other_routes = Router::new()
        .route("/auth/logout", post(auth::logout))
        .route("/auth/logout-all", post(auth::logout_all))
        .route("/auth/me", get(auth::me))
        .route("/auth/providers", get(auth::providers))
        .route("/sessions", post(sessions::create).get(sessions::list))
        .route("/sessions/{id}/hands", post(sessions::add_hand))
        .route("/sessions/{id}/stats", get(sessions::stats))
        .route("/sessions/{id}/end", post(sessions::end))
        .route("/health", get(routes::health));

    let api = analyze_routes
        .merge(auth_routes)
        .merge(other_routes)
        .with_state(state);

    let origin = cfg
        .frontend_origin
        .parse::<HeaderValue>()
        .expect("FRONTEND_ORIGIN debe ser un origen válido");
    let cors = CorsLayer::new()
        .allow_origin(origin)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH])
        .allow_headers([header::CONTENT_TYPE]);

    Router::new()
        .nest("/api/v1", api)
        .fallback(not_found)
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
