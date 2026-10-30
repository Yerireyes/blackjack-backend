//! `/play/*`: rondas jugadas en el servidor. El servidor guarda el zapato, reparte,
//! valida cada acción, juega el dealer y liquida; el cliente solo manda intenciones.
//!
//! Cada operación corre en una transacción que primero bloquea la fila del usuario
//! (y luego la partida o cuenta y la ronda): una sola ronda activa por usuario y
//! claves de idempotencia seguras aunque lleguen en paralelo.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use engine::game::{
    GameError, GameShoe, NewRoundRequest, Round, RoundActionRequest, RoundKind, RoundPhase,
    RoundResult, RoundView, WalletView,
};
use engine::{Action, Rules};
use sqlx::types::Json as Jsonb;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::analyze::run_engine;
use crate::routes::casual::{
    self, has_active_round, lock_active_run, lock_run, lock_user, no_active_run,
    require_display_name, round_active, run_finished, shuffled_shoe, Run, BET_STEP, MAX_BET,
    MIN_BET,
};
use crate::routes::daily::{lock_attempt, lock_today_attempt, Attempt};
use crate::routes::ranked::{self, lock_account, lock_current_account, refill_pending, Account};
use crate::state::AppState;

/// Ronda sin acciones durante este tiempo: se cierra plantándose.
const STALE_AFTER: time::Duration = time::Duration::hours(24);

/// De dónde sale y adónde va el dinero de la ronda.
enum Wallet {
    Casual(Run),
    Ranked(Account),
    Daily(Attempt),
}

impl Wallet {
    fn kind(&self) -> RoundKind {
        match self {
            Wallet::Casual(_) => RoundKind::Casual,
            Wallet::Ranked(_) => RoundKind::Ranked,
            Wallet::Daily(_) => RoundKind::Daily,
        }
    }

    /// Modos competitivos: sin ayudas durante la mano y con revisión al liquidar.
    fn competitive(&self) -> bool {
        !matches!(self, Wallet::Casual(_))
    }

    fn id(&self) -> Uuid {
        match self {
            Wallet::Casual(r) => r.id,
            Wallet::Ranked(a) => a.id,
            Wallet::Daily(d) => d.id,
        }
    }

    fn rules(&self) -> &Rules {
        match self {
            Wallet::Casual(r) => &r.rules,
            Wallet::Ranked(a) => &a.rules,
            Wallet::Daily(d) => &d.rules,
        }
    }

    fn shoe(&self) -> &GameShoe {
        match self {
            Wallet::Casual(r) => &r.shoe,
            Wallet::Ranked(a) => &a.shoe,
            Wallet::Daily(d) => &d.shoe,
        }
    }

    fn balance(&self) -> i64 {
        match self {
            Wallet::Casual(r) => r.balance,
            Wallet::Ranked(a) => a.balance,
            Wallet::Daily(d) => d.balance,
        }
    }

    /// Zapato y reglas a la vez (el zapato se modifica mientras se leen las reglas).
    fn table(&mut self) -> (&mut GameShoe, &Rules) {
        match self {
            Wallet::Casual(r) => (&mut r.shoe, &r.rules),
            Wallet::Ranked(a) => (&mut a.shoe, &a.rules),
            Wallet::Daily(d) => (&mut d.shoe, &d.rules),
        }
    }

    /// Rebaraja antes de repartir si hace falta. El reto diario usa la barajada del
    /// día (la misma para todos); los demás, el CSPRNG.
    fn shuffle_if_needed(&mut self, daily_seed: &[u8]) {
        let rules = self.rules();
        if !self
            .shoe()
            .needs_shuffle(rules.penetration, rules.max_split_hands)
        {
            return;
        }
        match self {
            Wallet::Daily(d) => d.reshuffle(daily_seed),
            _ => {
                let (shoe, rules) = self.table();
                *shoe = shuffled_shoe(rules.decks);
            }
        }
    }

    /// Aplica el neto de la ronda. Devuelve la recarga de Ranked, si hubo.
    fn settle(&mut self, net: i64) -> Option<i64> {
        match self {
            Wallet::Casual(r) => {
                r.settle(net);
                None
            }
            Wallet::Ranked(a) => a.settle(net),
            Wallet::Daily(d) => {
                d.settle(net);
                None
            }
        }
    }

    fn wallet_view(&self) -> WalletView {
        match self {
            Wallet::Casual(r) => WalletView {
                balance: r.balance,
                max_bet: if r.is_active() {
                    casual::max_bet(r.balance)
                } else {
                    0
                },
                refill_available_at: None,
            },
            Wallet::Ranked(a) => a.wallet_view(),
            Wallet::Daily(d) => WalletView {
                balance: d.balance,
                max_bet: d.max_bet(),
                refill_available_at: None,
            },
        }
    }

    async fn save(&mut self, db: &mut PgConnection) -> ApiResult<()> {
        match self {
            Wallet::Casual(r) => r.save(db).await,
            Wallet::Ranked(a) => a.save(db).await,
            Wallet::Daily(d) => d.save(db).await,
        }
    }
}

/// La partida, cuenta o intento `id` de la ronda, bloqueado.
async fn lock_wallet(
    db: &mut PgConnection,
    user_id: Uuid,
    kind: &str,
    id: Uuid,
) -> ApiResult<Wallet> {
    let wallet = match kind {
        "casual" => lock_run(db, user_id, id).await?.map(Wallet::Casual),
        "daily" => lock_attempt(db, user_id, id).await?.map(Wallet::Daily),
        _ => lock_account(db, user_id, id).await?.map(Wallet::Ranked),
    };
    wallet.ok_or_else(|| ApiError::internal("ronda sin partida, cuenta ni intento"))
}

fn kind_str(kind: RoundKind) -> &'static str {
    match kind {
        RoundKind::Casual => "casual",
        RoundKind::Ranked => "ranked",
        RoundKind::Daily => "daily",
    }
}

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

/// Lo que ve el cliente. `wallet` ya refleja la liquidación si la ronda terminó;
/// `refilled` es la recarga de esa liquidación.
fn round_view(
    id: Uuid,
    bet: i64,
    round: &Round,
    wallet: &Wallet,
    refilled: Option<i64>,
) -> RoundView {
    let settled = round.phase == RoundPhase::Settled;
    let competitive = wallet.competitive();
    let shoe = wallet.shoe();
    RoundView {
        id: id.to_string(),
        kind: wallet.kind(),
        context_id: wallet.id().to_string(),
        bet,
        phase: round.phase,
        player: round.player_view(bet),
        dealer: round.dealer_view(),
        allowed: round.allowed(wallet.rules(), spare_units(wallet.balance(), bet, round)),
        peek_blackjack: round.peek_blackjack,
        wallet: wallet.wallet_view(),
        // Sin ayudas en Ranked ni en el reto: la composición solo la ve Casual.
        shoe_counts: (!competitive).then(|| round.unseen_counts(shoe)),
        penetration: shoe.dealt_fraction(),
        result: settled.then(|| RoundResult {
            net: round.net_chips(bet),
            refilled,
            review: competitive.then(|| round.decisions.clone()),
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

fn bet_out_of_range(max: i64) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "bet_out_of_range",
        format!("la apuesta debe ser de {MIN_BET} a {max} en múltiplos de {BET_STEP}"),
    )
}

fn check_bet(bet: i64, wallet: &Wallet) -> ApiResult<()> {
    let valid = |max: i64| (MIN_BET..=max).contains(&bet) && bet % BET_STEP == 0;
    match wallet {
        Wallet::Casual(_) | Wallet::Daily(_) => {
            if !valid(MAX_BET) {
                return Err(bet_out_of_range(MAX_BET));
            }
            if bet > wallet.balance() {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "insufficient_funds",
                    "no te alcanza el saldo para esa apuesta",
                ));
            }
        }
        Wallet::Ranked(account) => {
            if account.max_bet() == 0 {
                return Err(refill_pending(account));
            }
            if !valid(account.max_bet()) {
                return Err(bet_out_of_range(account.max_bet()));
            }
        }
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
    let context = match req.kind {
        RoundKind::Ranked | RoundKind::Daily => None,
        RoundKind::Casual => req
            .context_id
            .as_deref()
            .map(|c| {
                c.parse::<Uuid>()
                    .map_err(|_| ApiError::bad_request("contextId debe ser un UUID"))
            })
            .transpose()?,
    };

    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    if let Some(view) = replayed_deal(&mut tx, user.id, key).await? {
        return Ok((StatusCode::CREATED, Json(view)));
    }
    let mut wallet = match req.kind {
        RoundKind::Casual => {
            let run = match context {
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
            Wallet::Casual(run)
        }
        RoundKind::Ranked => {
            // El nombre sale en el ranking.
            require_display_name(&mut tx, user.id).await?;
            Wallet::Ranked(lock_current_account(&mut tx, user.id).await?)
        }
        // El nombre ya se exigió al empezar el intento.
        RoundKind::Daily => Wallet::Daily(lock_today_attempt(&mut tx, user.id).await?),
    };
    check_bet(req.bet, &wallet)?;
    if has_active_round(&mut tx, user.id).await? {
        return Err(round_active());
    }

    wallet.shuffle_if_needed(&state.daily_seed);
    let (shoe, rules) = wallet.table();
    let round = Round::deal(shoe, rules).map_err(game_error)?;
    let settled = round.phase == RoundPhase::Settled;
    let refilled = if settled {
        wallet.settle(round.net_chips(req.bet))
    } else {
        None
    };
    let id = Uuid::new_v4();
    let view = round_view(id, req.bet, &round, &wallet, refilled);
    let now = OffsetDateTime::now_utc();
    sqlx::query!(
        "INSERT INTO rounds (id, user_id, kind, context_id, bet, state, status, net,
                             idempotency_key, response, created_at, settled_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        id,
        user.id,
        kind_str(wallet.kind()),
        wallet.id(),
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
    wallet.save(&mut tx).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view)))
}

struct RoundRow {
    id: Uuid,
    kind: String,
    context_id: Uuid,
    bet: i64,
    state: Jsonb<Round>,
    status: String,
}

/// La ronda `id` del usuario, bloqueada.
async fn lock_round(db: &mut PgConnection, user_id: Uuid, id: Uuid) -> ApiResult<Option<RoundRow>> {
    let row = sqlx::query_as!(
        RoundRow,
        r#"SELECT id, kind, context_id, bet, state AS "state: Jsonb<Round>", status
           FROM rounds WHERE id = $1 AND user_id = $2 FOR UPDATE"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row)
}

/// Aplica `action` a la ronda, la guarda junto con la cartera y registra la acción
/// con su respuesta.
async fn apply_action(
    db: &mut PgConnection,
    row: &RoundRow,
    round: &mut Round,
    wallet: &mut Wallet,
    action: Action,
    key: Uuid,
) -> ApiResult<RoundView> {
    let spare = spare_units(wallet.balance(), row.bet, round);
    let (shoe, rules) = wallet.table();
    round.act(action, shoe, rules, spare).map_err(game_error)?;
    let settled = round.phase == RoundPhase::Settled;
    let refilled = if settled {
        wallet.settle(round.net_chips(row.bet))
    } else {
        None
    };
    let view = round_view(row.id, row.bet, round, wallet, refilled);
    sqlx::query!(
        "UPDATE rounds SET state = $2, status = $3, net = $4, settled_at = $5 WHERE id = $1",
        row.id,
        Jsonb(&*round) as _,
        if settled { "settled" } else { "active" },
        settled.then(|| round.net_chips(row.bet)),
        settled.then(OffsetDateTime::now_utc)
    )
    .execute(&mut *db)
    .await?;
    sqlx::query!(
        "INSERT INTO round_actions (id, round_id, seq, action, idempotency_key, response)
         VALUES ($1, $2, (SELECT count(*) + 1 FROM round_actions WHERE round_id = $2), $3, $4, $5)",
        Uuid::new_v4(),
        row.id,
        action_str(action),
        key,
        Jsonb(&view) as _
    )
    .execute(&mut *db)
    .await?;
    wallet.save(db).await?;
    Ok(view)
}

/// Ranked y reto diario: evalúa la decisión con `analyze_hand` antes de aplicarla, con
/// lo que el jugador veía. Si el cálculo falla, la acción sigue y esa decisión no se
/// revisa.
async fn review_decision(round: &mut Round, wallet: &Wallet, bet: i64, action: Action) {
    if !wallet.competitive() {
        return;
    }
    let spare = spare_units(wallet.balance(), bet, round);
    if !round.allowed(wallet.rules(), spare).get(action) {
        return;
    }
    let Some(req) = round.analysis_request(wallet.rules(), wallet.shoe(), spare) else {
        return;
    };
    match run_engine(move || engine::analyze_hand(req)).await {
        Ok((analysis, _)) => round.record_decision(action, &analysis),
        Err(e) => tracing::warn!("no se pudo revisar la decisión: {}", e.message),
    }
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
    let row = lock_round(&mut tx, user.id, round_id)
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
    let mut wallet = lock_wallet(&mut tx, user.id, &row.kind, row.context_id).await?;
    let mut round = row.state.0.clone();
    review_decision(&mut round, &wallet, row.bet, req.action).await;
    let view = apply_action(&mut tx, &row, &mut round, &mut wallet, req.action, key).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// `GET /play/active`: la ronda en juego, para retomarla tras una desconexión.
pub async fn active(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<RoundView>> {
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as!(
        RoundRow,
        r#"SELECT id, kind, context_id, bet, state AS "state: Jsonb<Round>", status
           FROM rounds WHERE user_id = $1 AND status = 'active'"#,
        user.id
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(no_active_round)?;
    // Lectura: el bloqueo se libera al terminar la transacción sin cambios.
    let wallet = lock_wallet(&mut tx, user.id, &row.kind, row.context_id).await?;
    Ok(Json(round_view(
        row.id,
        row.bet,
        &row.state.0,
        &wallet,
        None,
    )))
}

/// Cierra las rondas sin acciones durante 24 h plantándose en todas las manos
/// pendientes (sin revisión). Devuelve cuántas cerró.
pub async fn close_stale_rounds(pool: &PgPool) -> ApiResult<u64> {
    let stale = sqlx::query!(
        "SELECT r.id, r.user_id FROM rounds r
         WHERE r.status = 'active'
           AND coalesce((SELECT max(a.created_at) FROM round_actions a WHERE a.round_id = r.id),
                        r.created_at) < $1",
        OffsetDateTime::now_utc() - STALE_AFTER
    )
    .fetch_all(pool)
    .await?;
    let mut closed = 0;
    for r in stale {
        let mut tx = pool.begin().await?;
        lock_user(&mut tx, r.user_id).await?;
        let Some(row) = lock_round(&mut tx, r.user_id, r.id).await? else {
            continue;
        };
        if row.status != "active" {
            continue;
        }
        let mut wallet = lock_wallet(&mut tx, r.user_id, &row.kind, row.context_id).await?;
        let mut round = row.state.0.clone();
        while round.phase == RoundPhase::Player {
            apply_action(
                &mut tx,
                &row,
                &mut round,
                &mut wallet,
                Action::Stand,
                Uuid::new_v4(),
            )
            .await?;
        }
        tx.commit().await?;
        closed += 1;
    }
    Ok(closed)
}

/// Cierre periódico: rondas abandonadas y temporadas terminadas.
pub async fn housekeeping(pool: &PgPool) {
    match close_stale_rounds(pool).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("rondas abandonadas cerradas: {n}"),
        Err(e) => tracing::warn!(
            "no se pudieron cerrar las rondas abandonadas: {}",
            e.message
        ),
    }
    match ranked::close_seasons(pool).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("temporadas ranked cerradas: {n}"),
        Err(e) => tracing::warn!("no se pudieron cerrar las temporadas: {e}"),
    }
}
