//! `/casual/*` y `/me/records`: partidas Casual (500 fichas, reglas elegidas),
//! ranking semanal por mejor pico y récords personales. Las rondas se juegan en
//! `/play/*` ([`super::play`]).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use engine::game::{
    CasualRecords, CasualRunStatus, CasualRunView, CreateCasualRunRequest, GameShoe,
    LeaderboardRow, MyRecords, Preset, WeeklyLeaderboard,
};
use engine::Rules;
use rand::RngExt;
use sqlx::types::Json as Jsonb;
use sqlx::PgConnection;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, Time};
use uuid::Uuid;

use crate::auth::{current_session, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::routes::auth::unique_violation;
use crate::routes::cosmetics::title;
use crate::state::AppState;

pub const START_BALANCE: i64 = 500;
pub const MIN_BET: i64 = 10;
pub const MAX_BET: i64 = 500;
pub const BET_STEP: i64 = 10;

/// Apuesta máxima con este saldo: hasta 500 y nunca más que el saldo, en múltiplos de 10.
pub fn max_bet(balance: i64) -> i64 {
    if balance < MIN_BET {
        0
    } else {
        balance.min(MAX_BET) / BET_STEP * BET_STEP
    }
}

/// Zapato nuevo barajado con el CSPRNG del sistema (ChaCha sembrado desde el SO).
pub fn shuffled_shoe(decks: u8) -> GameShoe {
    let mut rng = rand::rng();
    GameShoe::shuffled(decks, |n| rng.random_range(0..n))
}

pub fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).expect("fecha formateable")
}

/// Ahora, truncado a microsegundos como lo guarda Postgres: así la respuesta que crea
/// un registro coincide con la que lo lee después.
pub fn now_micros() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .expect("nanosegundos válidos")
}

/// Partida casual tal como está en la base.
pub struct Run {
    pub id: Uuid,
    pub rules: Rules,
    pub preset: Option<Preset>,
    pub shoe: GameShoe,
    pub balance: i64,
    pub peak: i64,
    pub hands: i32,
    pub status: CasualRunStatus,
    pub started_at: OffsetDateTime,
    pub ended_at: Option<OffsetDateTime>,
}

struct RunRow {
    id: Uuid,
    rules: Jsonb<Rules>,
    preset: Option<String>,
    shoe: Jsonb<GameShoe>,
    balance: i64,
    peak: i64,
    hands: i32,
    status: String,
    started_at: OffsetDateTime,
    ended_at: Option<OffsetDateTime>,
}

fn preset_str(p: Preset) -> &'static str {
    match p {
        Preset::Vegas => "vegas",
        Preset::Europe => "europe",
        Preset::SixToFive => "sixToFive",
    }
}

fn status_str(s: CasualRunStatus) -> &'static str {
    match s {
        CasualRunStatus::Active => "active",
        CasualRunStatus::Retired => "retired",
        CasualRunStatus::Busted => "busted",
    }
}

impl TryFrom<RunRow> for Run {
    type Error = ApiError;
    fn try_from(r: RunRow) -> ApiResult<Run> {
        let preset = match r.preset.as_deref() {
            None => None,
            Some("vegas") => Some(Preset::Vegas),
            Some("europe") => Some(Preset::Europe),
            Some("sixToFive") => Some(Preset::SixToFive),
            Some(other) => return Err(ApiError::internal(format!("preset desconocido: {other}"))),
        };
        let status = match r.status.as_str() {
            "active" => CasualRunStatus::Active,
            "retired" => CasualRunStatus::Retired,
            "busted" => CasualRunStatus::Busted,
            other => return Err(ApiError::internal(format!("estado desconocido: {other}"))),
        };
        Ok(Run {
            id: r.id,
            rules: r.rules.0,
            preset,
            shoe: r.shoe.0,
            balance: r.balance,
            peak: r.peak,
            hands: r.hands,
            status,
            started_at: r.started_at,
            ended_at: r.ended_at,
        })
    }
}

impl Run {
    pub fn is_active(&self) -> bool {
        self.status == CasualRunStatus::Active
    }

    pub fn view(&self) -> CasualRunView {
        CasualRunView {
            id: self.id.to_string(),
            rules: self.rules.clone(),
            preset: self.preset,
            status: self.status,
            balance: self.balance,
            peak: self.peak,
            hands: self.hands.max(0) as u32,
            max_bet: if self.is_active() {
                max_bet(self.balance)
            } else {
                0
            },
            penetration: self.shoe.dealt_fraction(),
            started_at: rfc3339(self.started_at),
            ended_at: self.ended_at.map(rfc3339),
        }
    }

    /// Aplica el neto de una ronda liquidada: saldo, pico, manos y, si el saldo queda
    /// por debajo de la apuesta mínima, la partida termina (`busted`).
    pub fn settle(&mut self, net: i64) {
        self.balance = (self.balance + net).max(0);
        self.peak = self.peak.max(self.balance);
        self.hands += 1;
        if self.balance < MIN_BET {
            self.status = CasualRunStatus::Busted;
            self.ended_at = Some(now_micros());
        }
    }

    /// Guarda zapato, saldo, pico, manos y estado.
    pub async fn save(&self, db: &mut PgConnection) -> ApiResult<()> {
        sqlx::query!(
            "UPDATE casual_runs
             SET shoe = $2, balance = $3, peak = $4, hands = $5, status = $6, ended_at = $7
             WHERE id = $1",
            self.id,
            Jsonb(&self.shoe) as _,
            self.balance,
            self.peak,
            self.hands,
            status_str(self.status),
            self.ended_at
        )
        .execute(db)
        .await?;
        Ok(())
    }
}

/// La partida `id` del usuario, bloqueada para esta transacción.
pub async fn lock_run(db: &mut PgConnection, user_id: Uuid, id: Uuid) -> ApiResult<Option<Run>> {
    let row = sqlx::query_as!(
        RunRow,
        r#"SELECT id, rules AS "rules: Jsonb<Rules>", preset, shoe AS "shoe: Jsonb<GameShoe>",
                  balance, peak, hands, status, started_at, ended_at
           FROM casual_runs WHERE id = $1 AND user_id = $2 FOR UPDATE"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await?;
    row.map(Run::try_from).transpose()
}

/// La partida activa del usuario, bloqueada para esta transacción.
pub async fn lock_active_run(db: &mut PgConnection, user_id: Uuid) -> ApiResult<Option<Run>> {
    let row = sqlx::query_as!(
        RunRow,
        r#"SELECT id, rules AS "rules: Jsonb<Rules>", preset, shoe AS "shoe: Jsonb<GameShoe>",
                  balance, peak, hands, status, started_at, ended_at
           FROM casual_runs WHERE user_id = $1 AND status = 'active' FOR UPDATE"#,
        user_id
    )
    .fetch_optional(db)
    .await?;
    row.map(Run::try_from).transpose()
}

/// Bloquea la fila del usuario: serializa todas sus operaciones de juego (crear o
/// cerrar partidas, repartir, actuar), así una clave de idempotencia repetida en
/// paralelo ve el resultado de la primera.
pub async fn lock_user(db: &mut PgConnection, user_id: Uuid) -> ApiResult<()> {
    sqlx::query!("SELECT id FROM users WHERE id = $1 FOR UPDATE", user_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(())
}

pub fn no_active_run() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "no_active_run",
        "no hay una partida casual activa",
    )
}

pub fn run_finished() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "run_finished",
        "la partida ya terminó",
    )
}

pub fn round_active() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "round_active",
        "hay una ronda sin terminar",
    )
}

/// El nombre es público en los rankings: no se juega con el provisional.
pub async fn require_display_name(db: &mut PgConnection, user_id: Uuid) -> ApiResult<()> {
    let name_set = sqlx::query_scalar!("SELECT display_name_set FROM users WHERE id = $1", user_id)
        .fetch_one(db)
        .await?;
    if !name_set {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "display_name_required",
            "elige tu nombre antes de jugar",
        ));
    }
    Ok(())
}

pub async fn has_active_round(db: &mut PgConnection, user_id: Uuid) -> ApiResult<bool> {
    let active = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM rounds WHERE user_id = $1 AND status = 'active') AS "active!""#,
        user_id
    )
    .fetch_one(db)
    .await?;
    Ok(active)
}

/// `POST /casual/runs`: nueva partida con 500 fichas y las reglas elegidas.
pub async fn create_run(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<CreateCasualRunRequest>,
) -> ApiResult<(StatusCode, Json<CasualRunView>)> {
    let (rules, preset) = match (req.preset, req.rules) {
        (Some(p), None) => (p.rules(), Some(p)),
        (None, Some(r)) => (r, None),
        _ => {
            return Err(ApiError::bad_request(
                "indica un preset o reglas personalizadas (uno de los dos)",
            ))
        }
    };
    rules.validate()?;

    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    require_display_name(&mut tx, user.id).await?;
    let run = Run {
        id: Uuid::new_v4(),
        shoe: shuffled_shoe(rules.decks),
        rules,
        preset,
        balance: START_BALANCE,
        peak: START_BALANCE,
        hands: 0,
        status: CasualRunStatus::Active,
        started_at: now_micros(),
        ended_at: None,
    };
    sqlx::query!(
        "INSERT INTO casual_runs (id, user_id, rules, preset, shoe, balance, peak, started_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        run.id,
        user.id,
        Jsonb(&run.rules) as _,
        run.preset.map(preset_str),
        Jsonb(&run.shoe) as _,
        run.balance,
        run.peak,
        run.started_at
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("casual_runs_one_active") => ApiError::new(
            StatusCode::CONFLICT,
            "run_active",
            "ya tienes una partida casual activa",
        ),
        _ => ApiError::from(e),
    })?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(run.view())))
}

/// `GET /casual/runs/active`.
pub async fn active_run(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<CasualRunView>> {
    let mut db = state.pool.acquire().await?;
    let row = sqlx::query_as!(
        RunRow,
        r#"SELECT id, rules AS "rules: Jsonb<Rules>", preset, shoe AS "shoe: Jsonb<GameShoe>",
                  balance, peak, hands, status, started_at, ended_at
           FROM casual_runs WHERE user_id = $1 AND status = 'active'"#,
        user.id
    )
    .fetch_optional(&mut *db)
    .await?
    .ok_or_else(no_active_run)?;
    Ok(Json(Run::try_from(row)?.view()))
}

/// `POST /casual/runs/{id}/retire`: el usuario se retira con el saldo actual.
pub async fn retire(
    State(state): State<AppState>,
    user: AuthUser,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<CasualRunView>> {
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let mut run = lock_run(&mut tx, user.id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("partida no encontrada"))?;
    if !run.is_active() {
        return Err(run_finished());
    }
    if has_active_round(&mut tx, user.id).await? {
        return Err(round_active());
    }
    run.status = CasualRunStatus::Retired;
    run.ended_at = Some(now_micros());
    run.save(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(run.view()))
}

/// Lunes 00:00 UTC de la semana de `now`.
pub fn week_start(now: OffsetDateTime) -> OffsetDateTime {
    let now = now.to_offset(time::UtcOffset::UTC);
    let days = now.weekday().number_days_from_monday() as i64;
    (now - Duration::days(days)).replace_time(Time::MIDNIGHT)
}

/// `GET /casual/leaderboard/weekly`: los 100 mejores picos de las partidas empezadas
/// esta semana (un pico por usuario). Sin sesión, `me` no viene e `isMe` es falso.
pub async fn weekly_leaderboard(
    State(state): State<AppState>,
    jar: CookieJar,
) -> ApiResult<Json<WeeklyLeaderboard>> {
    let me = current_session(&jar, &state).await?.map(|s| s.user_id);
    let start = week_start(OffsetDateTime::now_utc());
    let rows = sqlx::query!(
        r#"WITH best AS (
               SELECT user_id, max(peak) AS peak FROM casual_runs
               WHERE started_at >= $1 GROUP BY user_id
           ), ranked AS (
               SELECT b.user_id, b.peak, u.display_name, c.title,
                      rank() OVER (ORDER BY b.peak DESC) AS pos,
                      row_number() OVER (ORDER BY b.peak DESC, u.display_name_key) AS n
               FROM best b JOIN users u ON u.id = b.user_id
               LEFT JOIN user_cosmetics c ON c.user_id = b.user_id
               WHERE u.banned_at IS NULL
           )
           SELECT user_id, peak AS "peak!", display_name, title AS "title?", pos AS "pos!",
                  n AS "n!"
           FROM ranked WHERE n <= 100 OR user_id = $2
           ORDER BY n"#,
        start,
        me
    )
    .fetch_all(&state.pool)
    .await?;
    let mut top = Vec::new();
    let mut mine = None;
    for r in rows {
        let row = LeaderboardRow {
            rank: r.pos as u32,
            display_name: r.display_name,
            title: title(r.title.as_deref()),
            value: r.peak,
            is_me: Some(r.user_id) == me,
        };
        if row.is_me {
            mine = Some(row.clone());
        }
        if r.n <= 100 {
            top.push(row);
        }
    }
    Ok(Json(WeeklyLeaderboard {
        week_start: rfc3339(start),
        top,
        me: mine,
    }))
}

/// `GET /me/records`: récords personales de Casual y resultados de Ranked.
pub async fn records(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<MyRecords>> {
    let mut db = state.pool.acquire().await?;
    let r = sqlx::query!(
        r#"SELECT count(*) AS "runs!", max(peak) AS best_peak, max(hands) AS most_hands
           FROM casual_runs WHERE user_id = $1"#,
        user.id
    )
    .fetch_one(&mut *db)
    .await?;
    Ok(Json(MyRecords {
        casual: CasualRecords {
            runs: r.runs as u32,
            best_peak: r.best_peak,
            most_hands: r.most_hands.map(|h| h.max(0) as u32),
        },
        ranked: super::ranked::records(&mut db, user.id).await?,
        daily: super::daily::records(&mut db, user.id).await?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn max_bet_rounds_down_to_tens() {
        assert_eq!(max_bet(9), 0);
        assert_eq!(max_bet(10), 10);
        assert_eq!(max_bet(255), 250);
        assert_eq!(max_bet(10_000), 500);
    }

    #[test]
    fn week_starts_on_monday_utc() {
        assert_eq!(
            week_start(datetime!(2026-10-29 15:30 UTC)), // jueves
            datetime!(2026-10-26 00:00 UTC)
        );
        assert_eq!(
            week_start(datetime!(2026-10-26 00:00 UTC)),
            datetime!(2026-10-26 00:00 UTC)
        );
        assert_eq!(
            week_start(datetime!(2026-11-01 23:59 UTC)), // domingo
            datetime!(2026-10-26 00:00 UTC)
        );
    }
}
