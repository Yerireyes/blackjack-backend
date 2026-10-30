//! `/daily/*`: reto diario. Mismas reglas (Las Vegas) y mismo zapato para todos ese día
//! (UTC); un intento de 30 manos por usuario y día, con 500 fichas; ranking diario por
//! saldo final. Las rondas se juegan en `/play/*` ([`super::play`]).
//!
//! El zapato sale de `HMAC-SHA256(DAILY_SEED_SECRET, "AAAA-MM-DD:n")` (n = número de
//! barajada): la semilla es secreta para que no se pueda precalcular.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use engine::game::{
    DailyAttemptStatus, DailyAttemptView, DailyLeaderboard, DailyRecords, DailyRow, GameShoe,
    Preset,
};
use engine::Rules;
use hmac::{Hmac, Mac};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::Deserialize;
use sha2::Sha256;
use sqlx::types::Json as Jsonb;
use sqlx::PgConnection;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::auth::{current_session, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::auth::unique_violation;
use crate::routes::casual::{self, lock_user, now_micros, require_display_name, rfc3339, MIN_BET};
use crate::routes::cosmetics::title;
use crate::state::AppState;

pub const START_BALANCE: i64 = 500;
/// Manos de un intento.
pub const HANDS: i32 = 30;

/// Las Vegas para todos; no se eligen.
pub fn daily_rules() -> Rules {
    Preset::Vegas.rules()
}

pub fn today() -> Date {
    OffsetDateTime::now_utc().date()
}

/// El zapato del día tras `n` barajadas: igual para todos los que lo juegan.
pub fn daily_shoe(key: &[u8], day: Date, n: i32) -> GameShoe {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC acepta claves de cualquier largo");
    mac.update(format!("{day}:{n}").as_bytes());
    let seed: [u8; 32] = mac.finalize().into_bytes().into();
    let mut rng = ChaCha20Rng::from_seed(seed);
    GameShoe::shuffled(daily_rules().decks, |n| rng.random_range(0..n))
}

/// Intento del reto tal como está en la base.
pub struct Attempt {
    pub id: Uuid,
    pub day: Date,
    pub rules: Rules,
    pub shoe: GameShoe,
    pub shuffles: i32,
    pub balance: i64,
    pub peak: i64,
    pub hands: i32,
    pub status: DailyAttemptStatus,
    pub started_at: OffsetDateTime,
    pub finished_at: Option<OffsetDateTime>,
}

struct AttemptRow {
    id: Uuid,
    day: Date,
    shoe: Jsonb<GameShoe>,
    shuffles: i32,
    balance: i64,
    peak: i64,
    hands: i32,
    status: String,
    started_at: OffsetDateTime,
    finished_at: Option<OffsetDateTime>,
}

impl From<AttemptRow> for Attempt {
    fn from(r: AttemptRow) -> Attempt {
        Attempt {
            id: r.id,
            day: r.day,
            rules: daily_rules(),
            shoe: r.shoe.0,
            shuffles: r.shuffles,
            balance: r.balance,
            peak: r.peak,
            hands: r.hands,
            status: if r.status == "finished" {
                DailyAttemptStatus::Finished
            } else {
                DailyAttemptStatus::Active
            },
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
    }
}

impl Attempt {
    pub fn is_active(&self) -> bool {
        self.status == DailyAttemptStatus::Active
    }

    pub fn max_bet(&self) -> i64 {
        if self.is_active() {
            casual::max_bet(self.balance)
        } else {
            0
        }
    }

    /// Siguiente barajada del día (la misma para todos los que lleguen a ella).
    pub fn reshuffle(&mut self, key: &[u8]) {
        self.shuffles += 1;
        self.shoe = daily_shoe(key, self.day, self.shuffles);
    }

    /// Aplica el neto de una ronda liquidada. El intento termina a las 30 manos o con
    /// menos de la apuesta mínima.
    pub fn settle(&mut self, net: i64) {
        self.balance = (self.balance + net).max(0);
        self.peak = self.peak.max(self.balance);
        self.hands += 1;
        if self.hands >= HANDS || self.balance < MIN_BET {
            self.status = DailyAttemptStatus::Finished;
            self.finished_at = Some(now_micros());
        }
    }

    pub fn view(&self) -> DailyAttemptView {
        DailyAttemptView {
            id: self.id.to_string(),
            day: self.day.to_string(),
            status: self.status,
            balance: self.balance,
            peak: self.peak,
            hands: self.hands.max(0) as u32,
            hands_total: HANDS as u32,
            max_bet: self.max_bet(),
            penetration: self.shoe.dealt_fraction(),
            started_at: rfc3339(self.started_at),
            finished_at: self.finished_at.map(rfc3339),
        }
    }

    pub async fn save(&self, db: &mut PgConnection) -> ApiResult<()> {
        sqlx::query!(
            "UPDATE daily_attempts
             SET shoe = $2, shuffles = $3, balance = $4, peak = $5, hands = $6, status = $7,
                 finished_at = $8
             WHERE id = $1",
            self.id,
            Jsonb(&self.shoe) as _,
            self.shuffles,
            self.balance,
            self.peak,
            self.hands,
            if self.is_active() {
                "active"
            } else {
                "finished"
            },
            self.finished_at
        )
        .execute(db)
        .await?;
        Ok(())
    }
}

async fn find_attempt(
    db: &mut PgConnection,
    user_id: Uuid,
    day: Date,
    lock: bool,
) -> ApiResult<Option<Attempt>> {
    // Dos consultas fijas en vez de SQL dinámico: sqlx las comprueba al compilar.
    let row = if lock {
        sqlx::query_as!(
            AttemptRow,
            r#"SELECT id, day, shoe AS "shoe: Jsonb<GameShoe>", shuffles, balance, peak, hands,
                      status, started_at, finished_at
               FROM daily_attempts WHERE user_id = $1 AND day = $2 FOR UPDATE"#,
            user_id,
            day
        )
        .fetch_optional(db)
        .await?
    } else {
        sqlx::query_as!(
            AttemptRow,
            r#"SELECT id, day, shoe AS "shoe: Jsonb<GameShoe>", shuffles, balance, peak, hands,
                      status, started_at, finished_at
               FROM daily_attempts WHERE user_id = $1 AND day = $2"#,
            user_id,
            day
        )
        .fetch_optional(db)
        .await?
    };
    Ok(row.map(Attempt::from))
}

/// El intento `id` del usuario, bloqueado para esta transacción.
pub async fn lock_attempt(
    db: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> ApiResult<Option<Attempt>> {
    let row = sqlx::query_as!(
        AttemptRow,
        r#"SELECT id, day, shoe AS "shoe: Jsonb<GameShoe>", shuffles, balance, peak, hands,
                  status, started_at, finished_at
           FROM daily_attempts WHERE id = $1 AND user_id = $2 FOR UPDATE"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Attempt::from))
}

/// El intento de hoy del usuario, bloqueado, para repartir.
pub async fn lock_today_attempt(db: &mut PgConnection, user_id: Uuid) -> ApiResult<Attempt> {
    let attempt = find_attempt(db, user_id, today(), true)
        .await?
        .ok_or_else(no_daily_attempt)?;
    if !attempt.is_active() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "daily_finished",
            "ya terminaste el reto de hoy",
        ));
    }
    Ok(attempt)
}

pub fn no_daily_attempt() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "no_daily_attempt",
        "todavía no empezaste el reto de hoy",
    )
}

/// `POST /daily/attempts`: empieza el intento de hoy (uno por día).
pub async fn start(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<(StatusCode, Json<DailyAttemptView>)> {
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    // El nombre sale en el ranking diario.
    require_display_name(&mut tx, user.id).await?;
    let day = today();
    let attempt = Attempt {
        id: Uuid::new_v4(),
        day,
        rules: daily_rules(),
        shoe: daily_shoe(&state.daily_seed, day, 0),
        shuffles: 0,
        balance: START_BALANCE,
        peak: START_BALANCE,
        hands: 0,
        status: DailyAttemptStatus::Active,
        started_at: now_micros(),
        finished_at: None,
    };
    sqlx::query!(
        "INSERT INTO daily_attempts (id, user_id, day, shoe, balance, peak, started_at)
         VALUES ($1, $2, $3, $4, $5, $5, $6)",
        attempt.id,
        user.id,
        attempt.day,
        Jsonb(&attempt.shoe) as _,
        attempt.balance,
        attempt.started_at
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| match unique_violation(&e) {
        Some("daily_attempts_user_id_day_key") => ApiError::new(
            StatusCode::CONFLICT,
            "daily_taken",
            "ya jugaste el reto de hoy; vuelve mañana",
        ),
        _ => ApiError::from(e),
    })?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(attempt.view())))
}

/// `GET /daily/me`: el intento de hoy (en juego o terminado).
pub async fn me(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<DailyAttemptView>> {
    let mut db = state.pool.acquire().await?;
    let attempt = find_attempt(&mut db, user.id, today(), false)
        .await?
        .ok_or_else(no_daily_attempt)?;
    Ok(Json(attempt.view()))
}

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    day: Option<String>,
}

/// `GET /daily/leaderboard?day=AAAA-MM-DD`: los 100 mejores saldos finales del día entre
/// los intentos terminados, más la fila del usuario. Sin `day`, hoy. Es público: sin
/// sesión no viene `me`.
pub async fn leaderboard(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<LeaderboardQuery>,
) -> ApiResult<Json<DailyLeaderboard>> {
    let day = match q.day {
        None => today(),
        Some(s) => Date::parse(&s, format_description!("[year]-[month]-[day]"))
            .map_err(|_| ApiError::bad_request("day debe tener el formato AAAA-MM-DD"))?,
    };
    let me = current_session(&jar, &state).await?.map(|s| s.user_id);
    let rows = sqlx::query!(
        r#"WITH q AS (
               SELECT d.user_id, d.balance, d.hands, u.display_name, c.title,
                      rank() OVER (ORDER BY d.balance DESC) AS pos,
                      row_number() OVER (ORDER BY d.balance DESC, d.finished_at) AS n
               FROM daily_attempts d JOIN users u ON u.id = d.user_id
               LEFT JOIN user_cosmetics c ON c.user_id = d.user_id
               WHERE d.day = $1 AND d.status = 'finished'
           )
           SELECT user_id, balance, hands, display_name, title AS "title?", pos AS "pos!"
           FROM q WHERE n <= 100 ORDER BY n"#,
        day
    )
    .fetch_all(&state.pool)
    .await?;
    let top = rows
        .into_iter()
        .map(|r| DailyRow {
            rank: r.pos as u32,
            display_name: r.display_name,
            title: title(r.title.as_deref()),
            balance: r.balance,
            hands: r.hands.max(0) as u32,
            finished: true,
            is_me: Some(r.user_id) == me,
        })
        .collect();
    let mine = match me {
        None => None,
        Some(user_id) => sqlx::query!(
            r#"SELECT d.balance, d.hands, d.status, u.display_name, c.title AS "title?",
                      1 + (SELECT count(*) FROM daily_attempts o
                           WHERE o.day = d.day AND o.status = 'finished'
                             AND o.balance > d.balance) AS "pos!"
               FROM daily_attempts d JOIN users u ON u.id = d.user_id
               LEFT JOIN user_cosmetics c ON c.user_id = d.user_id
               WHERE d.day = $1 AND d.user_id = $2"#,
            day,
            user_id
        )
        .fetch_optional(&state.pool)
        .await?
        .map(|r| DailyRow {
            rank: r.pos as u32,
            display_name: r.display_name,
            title: title(r.title.as_deref()),
            balance: r.balance,
            hands: r.hands.max(0) as u32,
            finished: r.status == "finished",
            is_me: true,
        }),
    };
    Ok(Json(DailyLeaderboard {
        day: day.to_string(),
        top,
        me: mine,
    }))
}

/// Retos del usuario para `/me/records`. La mejor posición solo cuenta días ya
/// terminados (antes de hoy), cuando el ranking ya no cambia.
pub async fn records(db: &mut PgConnection, user_id: Uuid) -> ApiResult<DailyRecords> {
    let r = sqlx::query!(
        r#"SELECT count(*) AS "finished!", max(balance) AS best_balance
           FROM daily_attempts WHERE user_id = $1 AND status = 'finished'"#,
        user_id
    )
    .fetch_one(&mut *db)
    .await?;
    let best_rank = sqlx::query_scalar!(
        r#"SELECT min(pos) AS best FROM (
               SELECT user_id, rank() OVER (PARTITION BY day ORDER BY balance DESC) AS pos
               FROM daily_attempts
               WHERE status = 'finished' AND day < $2
                 AND day IN (SELECT day FROM daily_attempts WHERE user_id = $1)
           ) q WHERE user_id = $1"#,
        user_id,
        today()
    )
    .fetch_one(db)
    .await?;
    Ok(DailyRecords {
        finished: r.finished as u32,
        best_balance: r.best_balance,
        best_rank: best_rank.map(|p| p as u32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn same_day_same_shoe_and_secret_matters() {
        let key = b"clave-de-prueba-de-al-menos-32-bytes";
        let day = date!(2026 - 10 - 29);
        let shoe = daily_shoe(key, day, 0);
        assert_eq!(shoe, daily_shoe(key, day, 0));
        assert_ne!(shoe, daily_shoe(key, day, 1), "otra barajada del día");
        assert_ne!(shoe, daily_shoe(key, date!(2026 - 10 - 30), 0), "otro día");
        assert_ne!(
            shoe,
            daily_shoe(b"otra-clave-de-al-menos-32-bytes!!!", day, 0)
        );
        assert_eq!(shoe.remaining_counts().iter().sum::<u16>(), 312);
    }

    #[test]
    fn attempt_ends_after_thirty_hands_or_when_busted() {
        let mut a = Attempt {
            id: Uuid::nil(),
            day: date!(2026 - 10 - 29),
            rules: daily_rules(),
            shoe: GameShoe::ordered(6),
            shuffles: 0,
            balance: START_BALANCE,
            peak: START_BALANCE,
            hands: 0,
            status: DailyAttemptStatus::Active,
            started_at: OffsetDateTime::now_utc(),
            finished_at: None,
        };
        for _ in 0..29 {
            a.settle(10);
        }
        assert!(a.is_active());
        assert_eq!(a.max_bet(), 500);
        a.settle(-100);
        assert!(!a.is_active());
        assert_eq!((a.balance, a.peak, a.hands, a.max_bet()), (690, 790, 30, 0));

        a.status = DailyAttemptStatus::Active;
        a.hands = 3;
        a.settle(-685);
        assert_eq!(
            a.status,
            DailyAttemptStatus::Finished,
            "con 5 no alcanza la mínima"
        );
    }
}
