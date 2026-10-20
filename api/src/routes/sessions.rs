//! `/sessions/*`: sesiones de juego, manos registradas y estadísticas.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use engine::{Action, Rank, Rules};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::state::AppState;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Player,
    Dealer,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Player => "player",
            Mode::Dealer => "dealer",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandResult {
    Win,
    Push,
    Lose,
}

impl HandResult {
    fn as_str(self) -> &'static str {
        match self {
            HandResult::Win => "win",
            HandResult::Push => "push",
            HandResult::Lose => "lose",
        }
    }
}

fn action_str(a: Action) -> &'static str {
    match a {
        Action::Stand => "stand",
        Action::Hit => "hit",
        Action::Double => "double",
        Action::Split => "split",
        Action::Surrender => "surrender",
    }
}

fn cards(ranks: &[Rank]) -> Vec<i16> {
    ranks.iter().map(|&r| u8::from(r) as i16).collect()
}

#[derive(Deserialize)]
pub struct NewSession {
    pub rules: Rules,
    pub mode: Mode,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    pub id: Uuid,
    pub rules: serde_json::Value,
    pub mode: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(body): ApiJson<NewSession>,
) -> ApiResult<(StatusCode, Json<SessionDto>)> {
    body.rules.validate()?;
    let rules = serde_json::to_value(&body.rules).map_err(ApiError::internal)?;
    let session = sqlx::query_as!(
        SessionDto,
        r#"INSERT INTO game_sessions (id, user_id, rules, mode) VALUES ($1, $2, $3, $4)
           RETURNING id, rules, mode, started_at, ended_at"#,
        Uuid::new_v4(),
        user.id,
        rules,
        body.mode.as_str()
    )
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(session)))
}

/// Sesiones del usuario, las más recientes primero (máximo 100).
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<SessionDto>>> {
    let sessions = sqlx::query_as!(
        SessionDto,
        r#"SELECT id, rules, mode, started_at, ended_at FROM game_sessions
           WHERE user_id = $1 ORDER BY started_at DESC LIMIT 100"#,
        user.id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(sessions))
}

/// Sesión del usuario o 404 (no revela si existe para otro usuario).
async fn owned_session(state: &AppState, user: AuthUser, id: Uuid) -> ApiResult<SessionDto> {
    sqlx::query_as!(
        SessionDto,
        r#"SELECT id, rules, mode, started_at, ended_at FROM game_sessions
           WHERE id = $1 AND user_id = $2"#,
        id,
        user.id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("sesión no encontrada"))
}

pub async fn end(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SessionDto>> {
    owned_session(&state, user, id).await?;
    let session = sqlx::query_as!(
        SessionDto,
        r#"UPDATE game_sessions SET ended_at = coalesce(ended_at, now()) WHERE id = $1
           RETURNING id, rules, mode, started_at, ended_at"#,
        id
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(session))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewHand {
    pub player_cards: Vec<Rank>,
    pub dealer_up: Rank,
    pub dealer_final: Vec<Rank>,
    pub chosen_action: Action,
    pub best_action: Action,
    pub ev_chosen: f64,
    pub ev_best: f64,
    pub exact: bool,
    pub result: HandResult,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandCreated {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

pub async fn add_hand(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(hand): ApiJson<NewHand>,
) -> ApiResult<(StatusCode, Json<HandCreated>)> {
    let session = owned_session(&state, user, id).await?;
    if session.ended_at.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "session_ended",
            "la sesión ya terminó",
        ));
    }
    if hand.player_cards.len() < 2 || hand.player_cards.len() > 21 || hand.dealer_final.len() > 21 {
        return Err(ApiError::bad_request("cantidad de cartas inválida"));
    }
    if !hand.ev_chosen.is_finite() || !hand.ev_best.is_finite() {
        return Err(ApiError::bad_request(
            "evChosen y evBest deben ser números finitos",
        ));
    }
    let created = sqlx::query_as!(
        HandCreated,
        r#"INSERT INTO hands (id, session_id, player_cards, dealer_up, dealer_final, chosen_action,
                              best_action, ev_chosen, ev_best, exact, result)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING id, created_at"#,
        Uuid::new_v4(),
        id,
        &cards(&hand.player_cards),
        u8::from(hand.dealer_up) as i16,
        &cards(&hand.dealer_final),
        action_str(hand.chosen_action),
        action_str(hand.best_action),
        hand.ev_chosen,
        hand.ev_best,
        hand.exact,
        hand.result.as_str()
    )
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub hands: i64,
    pub optimal_decisions: i64,
    /// Porcentaje de decisiones óptimas (0–100); `null` si no hay manos.
    pub optimal_pct: Option<f64>,
    /// EV perdido acumulado: suma de `evBest − evChosen`.
    pub ev_lost: f64,
    /// Decisiones cuyo EV salió de una aproximación (`exact = false`).
    pub approximate_decisions: i64,
    pub wins: i64,
    pub pushes: i64,
    pub losses: i64,
}

pub async fn stats(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Stats>> {
    owned_session(&state, user, id).await?;
    let row = sqlx::query!(
        r#"SELECT count(*) AS "hands!",
                  count(*) FILTER (WHERE chosen_action = best_action) AS "optimal!",
                  coalesce(sum(ev_best - ev_chosen), 0) AS "ev_lost!",
                  count(*) FILTER (WHERE NOT exact) AS "approximate!",
                  count(*) FILTER (WHERE result = 'win') AS "wins!",
                  count(*) FILTER (WHERE result = 'push') AS "pushes!",
                  count(*) FILTER (WHERE result = 'lose') AS "losses!"
           FROM hands WHERE session_id = $1"#,
        id
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(Stats {
        hands: row.hands,
        optimal_decisions: row.optimal,
        optimal_pct: (row.hands > 0).then(|| 100.0 * row.optimal as f64 / row.hands as f64),
        ev_lost: row.ev_lost,
        approximate_decisions: row.approximate,
        wins: row.wins,
        pushes: row.pushes,
        losses: row.losses,
    }))
}
