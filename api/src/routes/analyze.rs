//! `/analyze/*`: mismo contrato que el binding WASM, llamando al crate `engine`.

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use axum::Json;
use engine::{
    AnalyzeDealerRequest, AnalyzeDealerResponse, AnalyzeHandRequest, AnalyzeHandResponse,
    AnalyzeRoundRequest, AnalyzeRoundResponse, EngineError,
};

use crate::error::{ApiError, ApiResult};
use crate::extract::ApiJson;

/// Tiempo máximo de un cálculo.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(5);

/// Corre el cálculo en un hilo bloqueante (no bloquea el servidor) con límite de
/// tiempo, y devuelve también los milisegundos que tardó.
pub(crate) async fn run_engine<T, F>(f: F) -> ApiResult<(T, f64)>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, EngineError> + Send + 'static,
{
    let task = tokio::task::spawn_blocking(move || {
        let t0 = Instant::now();
        let res = f();
        (res, t0.elapsed().as_secs_f64() * 1000.0)
    });
    match tokio::time::timeout(ENGINE_TIMEOUT, task).await {
        Ok(Ok((res, ms))) => Ok((res?, ms)),
        Ok(Err(join)) => Err(ApiError::internal(join)),
        // El hilo bloqueante termina por su cuenta; aquí solo se deja de esperar.
        Err(_) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "timeout",
            "el cálculo superó el límite de tiempo",
        )),
    }
}

pub async fn hand(
    ApiJson(req): ApiJson<AnalyzeHandRequest>,
) -> ApiResult<Json<AnalyzeHandResponse>> {
    let (mut res, ms) = run_engine(move || engine::analyze_hand(req)).await?;
    res.compute_ms = ms;
    Ok(Json(res))
}

pub async fn dealer(
    ApiJson(req): ApiJson<AnalyzeDealerRequest>,
) -> ApiResult<Json<AnalyzeDealerResponse>> {
    let (mut res, ms) = run_engine(move || engine::analyze_dealer(req)).await?;
    res.compute_ms = ms;
    Ok(Json(res))
}

pub async fn round(
    ApiJson(req): ApiJson<AnalyzeRoundRequest>,
) -> ApiResult<Json<AnalyzeRoundResponse>> {
    let (mut res, ms) = run_engine(move || engine::analyze_round(req)).await?;
    res.compute_ms = ms;
    for h in &mut res.hands {
        h.compute_ms = ms;
    }
    Ok(Json(res))
}
