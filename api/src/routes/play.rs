//! `/play/*`: rondas jugadas en el servidor. El servidor guarda el zapato, reparte,
//! valida cada acción, juega el dealer y liquida; el cliente solo manda intenciones.
//!
//! Cada operación corre en una transacción que primero bloquea la fila del usuario
//! (y luego la partida y la ronda): una sola ronda activa por usuario y claves de
//! idempotencia seguras aunque lleguen en paralelo.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use engine::game::{
    GameError, NewRoundRequest, Round, RoundActionRequest, RoundKind, RoundPhase, RoundResult,
    RoundView, WalletView,
};
use engine::Action;
use sqlx::types::Json as Jsonb;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::casual::{
    has_active_round, lock_active_run, lock_run, lock_user, max_bet, no_active_run, round_active,
    run_finished, shuffled_shoe, Run, BET_STEP, MAX_BET, MIN_BET,
};
use crate::state::AppState;

fn idempotency_key(raw: &str) -> ApiResult<Uuid> {
    raw.parse()
        .map_err(|_| ApiError::bad_request("idempotencyKey debe ser un UUID"))
}

fn no_active_round() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "no_active_round",
        "no hay una ronda en juego",
    )
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

/// Unidades extra que el saldo cubre con esta apuesta (doblar y dividir necesitan una).
fn spare_units(balance: i64, bet: i64, round: &Round) -> u32 {
    let free = balance - bet * round.staked_units() as i64;
    (free.max(0) / bet).min(u32::MAX as i64) as u32
}

/// Lo que ve el cliente. `run` ya refleja la liquidación si la ronda terminó.
fn round_view(id: Uuid, bet: i64, round: &Round, run: &Run) -> RoundView {
    let settled = round.phase == RoundPhase::Settled;
    RoundView {
        id: id.to_string(),
        kind: RoundKind::Casual,
        context_id: run.id.to_string(),
        bet,
        phase: round.phase,
        player: round.player_view(bet),
        dealer: round.dealer_view(),
        allowed: round.allowed(&run.rules, spare_units(run.balance, bet, round)),
        peek_blackjack: round.peek_blackjack,
        wallet: WalletView {
            balance: run.balance,
            max_bet: if run.is_active() {
                max_bet(run.balance)
            } else {
                0
            },
            refill_available_at: None,
        },
        shoe_counts: Some(round.unseen_counts(&run.shoe)),
        penetration: run.shoe.dealt_fraction(),
        result: settled.then(|| RoundResult {
            net: round.net_chips(bet),
            refilled: None,
            review: None,
        }),
    }
}

fn game_error(e: GameError) -> ApiError {
    match e {
        GameError::ActionNotAllowed => ApiError::new(
            StatusCode::CONFLICT,
            "action_not_allowed",
            "esa acción no está permitida ahora",
        ),
        GameError::ShoeEmpty => ApiError::internal("se acabaron las cartas del zapato"),
    }
}

fn check_bet(bet: i64, balance: i64) -> ApiResult<()> {
    if !(MIN_BET..=MAX_BET).contains(&bet) || bet % BET_STEP != 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bet_out_of_range",
            format!("la apuesta debe ser de {MIN_BET} a {MAX_BET} en múltiplos de {BET_STEP}"),
        ));
    }
    if bet > balance {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "insufficient_funds",
            "no te alcanza el saldo para esa apuesta",
        ));
    }
    Ok(())
}

/// Respuesta ya guardada para esta clave de idempotencia del usuario.
async fn replayed_deal(
    db: &mut PgConnection,
    user_id: Uuid,
    key: Uuid,
) -> ApiResult<Option<RoundView>> {
    let response = sqlx::query_scalar!(
        r#"SELECT response AS "response: Jsonb<RoundView>" FROM rounds
           WHERE user_id = $1 AND idempotency_key = $2"#,
        user_id,
        key
    )
    .fetch_optional(db)
    .await?;
    Ok(response.map(|r| r.0))
}

/// `POST /play/rounds`: apuesta y reparte. Si la ronda se resuelve en el reparto
/// (blackjack o peek), ya viene liquidada.
pub async fn deal(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<NewRoundRequest>,
) -> ApiResult<(StatusCode, Json<RoundView>)> {
    let key = idempotency_key(&req.idempotency_key)?;
    if req.kind == RoundKind::Ranked {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_available",
            "Ranked todavía no está disponible",
        ));
    }
    let context = req
        .context_id
        .as_deref()
        .map(|c| {
            c.parse::<Uuid>()
                .map_err(|_| ApiError::bad_request("contextId debe ser un UUID"))
        })
        .transpose()?;

    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    if let Some(view) = replayed_deal(&mut tx, user.id, key).await? {
        return Ok((StatusCode::CREATED, Json(view)));
    }
    let mut run = match context {
        Some(id) => lock_run(&mut tx, user.id, id)
            .await?
            .ok_or_else(|| ApiError::not_found("partida no encontrada"))?,
        None => lock_active_run(&mut tx, user.id)
            .await?
            .ok_or_else(no_active_run)?,
    };
    if !run.is_active() {
        return Err(run_finished());
    }
    check_bet(req.bet, run.balance)?;
    if has_active_round(&mut tx, user.id).await? {
        return Err(round_active());
    }

    if run
        .shoe
        .needs_shuffle(run.rules.penetration, run.rules.max_split_hands)
    {
        run.shoe = shuffled_shoe(run.rules.decks);
    }
    let round = Round::deal(&mut run.shoe, &run.rules).map_err(game_error)?;
    let settled = round.phase == RoundPhase::Settled;
    if settled {
        run.settle(round.net_chips(req.bet));
    }
    let id = Uuid::new_v4();
    let view = round_view(id, req.bet, &round, &run);
    let now = OffsetDateTime::now_utc();
    sqlx::query!(
        "INSERT INTO rounds (id, user_id, kind, context_id, bet, state, status, net,
                             idempotency_key, response, created_at, settled_at)
         VALUES ($1, $2, 'casual', $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        id,
        user.id,
        run.id,
        req.bet,
        Jsonb(&round) as _,
        if settled { "settled" } else { "active" },
        settled.then(|| round.net_chips(req.bet)),
        key,
        Jsonb(&view) as _,
        now,
        settled.then_some(now)
    )
    .execute(&mut *tx)
    .await?;
    run.save(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view)))
}

struct RoundRow {
    id: Uuid,
    context_id: Uuid,
    bet: i64,
    state: Jsonb<Round>,
    status: String,
}

/// `POST /play/rounds/{id}/actions`: pedir, plantarse, doblar, dividir o rendirse.
pub async fn act(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(round_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<RoundActionRequest>,
) -> ApiResult<Json<RoundView>> {
    let key = idempotency_key(&req.idempotency_key)?;
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let row = sqlx::query_as!(
        RoundRow,
        r#"SELECT id, context_id, bet, state AS "state: Jsonb<Round>", status
           FROM rounds WHERE id = $1 AND user_id = $2 FOR UPDATE"#,
        round_id,
        user.id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("ronda no encontrada"))?;

    let replay = sqlx::query_scalar!(
        r#"SELECT response AS "response: Jsonb<RoundView>" FROM round_actions
           WHERE round_id = $1 AND idempotency_key = $2"#,
        row.id,
        key
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(view) = replay {
        return Ok(Json(view.0));
    }
    if row.status != "active" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "no_active_round",
            "la ronda ya terminó",
        ));
    }
    let mut run = lock_run(&mut tx, user.id, row.context_id)
        .await?
        .ok_or_else(|| ApiError::internal("ronda sin partida"))?;
    let mut round = row.state.0;
    let spare = spare_units(run.balance, row.bet, &round);
    round
        .act(req.action, &mut run.shoe, &run.rules, spare)
        .map_err(game_error)?;
    let settled = round.phase == RoundPhase::Settled;
    if settled {
        run.settle(round.net_chips(row.bet));
    }
    let view = round_view(row.id, row.bet, &round, &run);
    sqlx::query!(
        "UPDATE rounds SET state = $2, status = $3, net = $4, settled_at = $5 WHERE id = $1",
        row.id,
        Jsonb(&round) as _,
        if settled { "settled" } else { "active" },
        settled.then(|| round.net_chips(row.bet)),
        settled.then(OffsetDateTime::now_utc)
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO round_actions (id, round_id, seq, action, idempotency_key, response)
         VALUES ($1, $2, (SELECT count(*) + 1 FROM round_actions WHERE round_id = $2), $3, $4, $5)",
        Uuid::new_v4(),
        row.id,
        action_str(req.action),
        key,
        Jsonb(&view) as _
    )
    .execute(&mut *tx)
    .await?;
    run.save(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// `GET /play/active`: la ronda en juego, para retomarla tras una desconexión.
pub async fn active(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<RoundView>> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as!(
        RoundRow,
        r#"SELECT id, context_id, bet, state AS "state: Jsonb<Round>", status
           FROM rounds WHERE user_id = $1 AND status = 'active'"#,
        user.id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(no_active_round)?;
    // Lectura: el bloqueo se libera al terminar la transacción sin cambios.
    let run = lock_run(&mut tx, user.id, row.context_id)
        .await?
        .ok_or_else(|| ApiError::internal("ronda sin partida"))?;
    Ok(Json(round_view(row.id, row.bet, &row.state.0, &run)))
}
