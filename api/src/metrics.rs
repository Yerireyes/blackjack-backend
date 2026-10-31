//! `GET /metrics`: métricas en el formato de texto de Prometheus, para un panel o una
//! alerta. Solo se sirven con `METRICS_TOKEN` configurado y enviado como
//! `Authorization: Bearer <token>`; sin token configurado, la ruta responde 404.

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub struct Metrics {
    started: Instant,
    /// Respuestas por clase: 1xx … 5xx.
    responses: [AtomicU64; 5],
    token: Option<String>,
}

impl Metrics {
    pub fn new(token: Option<String>) -> Metrics {
        Metrics {
            started: Instant::now(),
            responses: Default::default(),
            token,
        }
    }

    fn record(&self, status: StatusCode) {
        let class = (status.as_u16() / 100).clamp(1, 5) as usize - 1;
        self.responses[class].fetch_add(1, Ordering::Relaxed);
    }

    /// Compara en tiempo constante, para no filtrar el token por la duración.
    fn authorized(&self, headers: &HeaderMap) -> Option<bool> {
        let token = self.token.as_deref()?;
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let same = given.len() == token.len()
            && given
                .bytes()
                .zip(token.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        Some(same)
    }
}

/// Middleware: cuenta cada respuesta por su clase de estado.
pub async fn count(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let res = next.run(req).await;
    state.metrics.record(res.status());
    res
}

fn gauge(out: &mut String, name: &str, help: &str, value: impl std::fmt::Display) {
    let _ = writeln!(
        out,
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}"
    );
}

/// `GET /metrics`.
pub async fn serve(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    match state.metrics.authorized(&headers) {
        None => return Err(ApiError::not_found("ruta no encontrada")),
        Some(false) => return Err(ApiError::unauthorized()),
        Some(true) => {}
    }
    let rounds = sqlx::query!(
        r#"SELECT kind, count(*) AS "n!" FROM rounds WHERE status = 'active' GROUP BY kind"#
    )
    .fetch_all(&state.pool)
    .await?;
    let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM users"#)
        .fetch_one(&state.pool)
        .await?;
    let (rooms, seated, spectators) = state.rooms.stats();

    let mut out = String::new();
    gauge(
        &mut out,
        "bj_uptime_seconds",
        "Segundos desde que arrancó el proceso.",
        state.metrics.started.elapsed().as_secs(),
    );
    let _ = writeln!(
        out,
        "# HELP bj_http_responses_total Respuestas HTTP por clase de estado.\n\
         # TYPE bj_http_responses_total counter"
    );
    for (i, n) in state.metrics.responses.iter().enumerate() {
        let _ = writeln!(
            out,
            "bj_http_responses_total{{class=\"{}xx\"}} {}",
            i + 1,
            n.load(Ordering::Relaxed)
        );
    }
    gauge(&mut out, "bj_users", "Cuentas registradas.", users);
    let _ = writeln!(
        out,
        "# HELP bj_rounds_active Rondas en juego por modo.\n# TYPE bj_rounds_active gauge"
    );
    for kind in ["casual", "ranked", "daily"] {
        let n = rounds.iter().find(|r| r.kind == kind).map_or(0, |r| r.n);
        let _ = writeln!(out, "bj_rounds_active{{kind=\"{kind}\"}} {n}");
    }
    gauge(
        &mut out,
        "bj_rooms_open",
        "Salas multijugador abiertas.",
        rooms,
    );
    gauge(
        &mut out,
        "bj_room_seated",
        "Jugadores sentados en salas.",
        seated,
    );
    gauge(
        &mut out,
        "bj_room_spectators",
        "Espectadores conectados a salas.",
        spectators,
    );
    gauge(
        &mut out,
        "bj_db_pool_connections",
        "Conexiones abiertas a PostgreSQL.",
        state.pool.size(),
    );
    gauge(
        &mut out,
        "bj_db_pool_idle",
        "Conexiones libres a PostgreSQL.",
        state.pool.num_idle(),
    );
    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        Body::from(out),
    )
        .into_response())
}
