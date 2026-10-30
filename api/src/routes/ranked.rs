//! `/ranked/*`: Ranked por temporadas mensuales (UTC) con reglas de Las Vegas fijas.
//! Cada usuario tiene una cuenta por temporada que empieza con 500; el ranking se
//! ordena por ganancia neta y solo entra quien juega el mínimo de manos. Las rondas
//! se juegan en `/play/*` ([`super::play`]).

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use engine::game::{
    GameShoe, Medal, Preset, RankedAccountView, RankedLeaderboard, RankedRecords, RankedRow,
    SeasonResult, WalletView,
};
use engine::Rules;
use serde::Deserialize;
use sqlx::types::Json as Jsonb;
use sqlx::{PgConnection, PgPool};
use time::{Date, Duration, OffsetDateTime, Time};
use uuid::Uuid;

use crate::auth::{current_session, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::casual::{lock_user, rfc3339, shuffled_shoe, BET_STEP, MAX_BET, MIN_BET};
use crate::routes::cosmetics::title;
use crate::state::AppState;

pub const START_BALANCE: i64 = 500;
/// Piso: al bajar de aquí se recarga hasta este saldo, como mucho una vez cada 24 h.
pub const FLOOR: i64 = 100;
pub const REFILL_EVERY: Duration = Duration::hours(24);
/// Manos de la temporada para entrar en el ranking y recibir medallas.
pub const MIN_HANDS: i32 = 200;

/// Las Vegas para todos; no se eligen.
pub fn ranked_rules() -> Rules {
    Preset::Vegas.rules()
}

/// `clamp(floor(0.25 × saldo / 10) × 10, 10, 500)`, o 0 si el saldo no llega al mínimo.
pub fn max_bet(balance: i64) -> i64 {
    if balance < MIN_BET {
        0
    } else {
        (balance / 4 / BET_STEP * BET_STEP).clamp(MIN_BET, MAX_BET)
    }
}

/// Temporada que contiene `now`: id `"AAAA-MM"`, inicio y fin (día 1 a las 00:00 UTC).
pub fn season_bounds(now: OffsetDateTime) -> (String, OffsetDateTime, OffsetDateTime) {
    let date = now.to_offset(time::UtcOffset::UTC).date();
    let first = |y: i32, m: time::Month| {
        Date::from_calendar_date(y, m, 1)
            .expect("día 1 válido")
            .with_time(Time::MIDNIGHT)
            .assume_utc()
    };
    let start = first(date.year(), date.month());
    let end = match date.month() {
        time::Month::December => first(date.year() + 1, time::Month::January),
        m => first(date.year(), m.next()),
    };
    let id = format!("{:04}-{:02}", date.year(), date.month() as u8);
    (id, start, end)
}

struct Season {
    id: String,
    starts_at: OffsetDateTime,
    ends_at: OffsetDateTime,
    closed_at: Option<OffsetDateTime>,
}

/// La temporada actual; la crea si es la primera vez que alguien la usa.
async fn current_season(db: &mut PgConnection) -> ApiResult<Season> {
    let (id, starts_at, ends_at) = season_bounds(OffsetDateTime::now_utc());
    sqlx::query!(
        "INSERT INTO seasons (id, starts_at, ends_at) VALUES ($1, $2, $3)
         ON CONFLICT (id) DO NOTHING",
        id,
        starts_at,
        ends_at
    )
    .execute(&mut *db)
    .await?;
    find_season(db, &id)
        .await?
        .ok_or_else(|| ApiError::internal("temporada recién creada no encontrada"))
}

async fn find_season(db: &mut PgConnection, id: &str) -> ApiResult<Option<Season>> {
    let season = sqlx::query_as!(
        Season,
        "SELECT id, starts_at, ends_at, closed_at FROM seasons WHERE id = $1",
        id
    )
    .fetch_optional(db)
    .await?;
    Ok(season)
}

/// Cuenta ranked de una temporada tal como está en la base.
pub struct Account {
    pub id: Uuid,
    pub season_id: String,
    pub season_ends_at: OffsetDateTime,
    pub rules: Rules,
    pub shoe: GameShoe,
    pub balance: i64,
    pub peak: i64,
    pub hands: i32,
    pub refilled_total: i64,
    pub last_refill_at: Option<OffsetDateTime>,
    /// Recarga aplicada en esta operación, a registrar al guardar.
    refill: Option<i64>,
}

struct AccountRow {
    id: Uuid,
    season_id: String,
    ends_at: OffsetDateTime,
    shoe: Jsonb<GameShoe>,
    balance: i64,
    peak: i64,
    hands: i32,
    refilled_total: i64,
    last_refill_at: Option<OffsetDateTime>,
}

impl From<AccountRow> for Account {
    fn from(r: AccountRow) -> Account {
        Account {
            id: r.id,
            season_id: r.season_id,
            season_ends_at: r.ends_at,
            rules: ranked_rules(),
            shoe: r.shoe.0,
            balance: r.balance,
            peak: r.peak,
            hands: r.hands,
            refilled_total: r.refilled_total,
            last_refill_at: r.last_refill_at,
            refill: None,
        }
    }
}

impl Account {
    pub fn net(&self) -> i64 {
        self.balance - START_BALANCE - self.refilled_total
    }

    pub fn max_bet(&self) -> i64 {
        max_bet(self.balance)
    }

    /// Cuándo se puede volver a recargar, si el saldo está bajo el piso y todavía no se
    /// puede.
    pub fn refill_available_at(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        let at = self.last_refill_at? + REFILL_EVERY;
        (self.balance < FLOOR && at > now).then_some(at)
    }

    /// Recarga hasta el piso si el saldo está por debajo y pasaron 24 h desde la última
    /// recarga. Devuelve lo recargado.
    pub fn refill_if_due(&mut self, now: OffsetDateTime) -> Option<i64> {
        if self.balance >= FLOOR || self.refill_available_at(now).is_some() {
            return None;
        }
        let amount = FLOOR - self.balance;
        self.balance = FLOOR;
        self.refilled_total += amount;
        self.last_refill_at = Some(now);
        self.refill = Some(self.refill.unwrap_or(0) + amount);
        Some(amount)
    }

    /// Aplica el neto de una ronda liquidada y, si hace falta y se puede, recarga.
    /// Devuelve la recarga aplicada.
    pub fn settle(&mut self, net: i64) -> Option<i64> {
        self.balance = (self.balance + net).max(0);
        self.peak = self.peak.max(self.balance);
        self.hands += 1;
        self.refill_if_due(OffsetDateTime::now_utc())
    }

    pub fn wallet_view(&self) -> WalletView {
        WalletView {
            balance: self.balance,
            max_bet: self.max_bet(),
            refill_available_at: self
                .refill_available_at(OffsetDateTime::now_utc())
                .map(rfc3339),
        }
    }

    pub fn view(&self) -> RankedAccountView {
        let wallet = self.wallet_view();
        RankedAccountView {
            id: self.id.to_string(),
            season: self.season_id.clone(),
            season_ends_at: rfc3339(self.season_ends_at),
            balance: self.balance,
            peak: self.peak,
            hands: self.hands.max(0) as u32,
            refilled_total: self.refilled_total,
            net: self.net(),
            max_bet: wallet.max_bet,
            refill_available_at: wallet.refill_available_at,
            min_hands: MIN_HANDS as u32,
            penetration: self.shoe.dealt_fraction(),
        }
    }

    /// Guarda zapato, saldo, pico, manos y recargas (y registra la recarga nueva).
    pub async fn save(&mut self, db: &mut PgConnection) -> ApiResult<()> {
        sqlx::query!(
            "UPDATE ranked_accounts
             SET shoe = $2, balance = $3, peak = $4, hands = $5, refilled_total = $6,
                 last_refill_at = $7
             WHERE id = $1",
            self.id,
            Jsonb(&self.shoe) as _,
            self.balance,
            self.peak,
            self.hands,
            self.refilled_total,
            self.last_refill_at
        )
        .execute(&mut *db)
        .await?;
        if let Some(amount) = self.refill.take() {
            sqlx::query!(
                "INSERT INTO ranked_refills (id, account_id, amount) VALUES ($1, $2, $3)",
                Uuid::new_v4(),
                self.id,
                amount
            )
            .execute(db)
            .await?;
        }
        Ok(())
    }
}

/// La cuenta `id` del usuario, bloqueada para esta transacción.
pub async fn lock_account(
    db: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> ApiResult<Option<Account>> {
    let row = sqlx::query_as!(
        AccountRow,
        r#"SELECT a.id, a.season_id, s.ends_at, a.shoe AS "shoe: Jsonb<GameShoe>", a.balance,
                  a.peak, a.hands, a.refilled_total, a.last_refill_at
           FROM ranked_accounts a JOIN seasons s ON s.id = a.season_id
           WHERE a.id = $1 AND a.user_id = $2 FOR UPDATE OF a"#,
        id,
        user_id
    )
    .fetch_optional(db)
    .await?;
    Ok(row.map(Account::from))
}

/// La cuenta del usuario en la temporada actual, bloqueada; la crea con 500 si es su
/// primera vez en la temporada. Aplica la recarga si toca. Requiere [`lock_user`].
pub async fn lock_current_account(db: &mut PgConnection, user_id: Uuid) -> ApiResult<Account> {
    let season = current_season(db).await?;
    let id = sqlx::query_scalar!(
        "SELECT id FROM ranked_accounts WHERE user_id = $1 AND season_id = $2",
        user_id,
        season.id
    )
    .fetch_optional(&mut *db)
    .await?;
    let id = match id {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4();
            sqlx::query!(
                "INSERT INTO ranked_accounts (id, user_id, season_id, shoe, balance, peak)
                 VALUES ($1, $2, $3, $4, $5, $5)",
                id,
                user_id,
                season.id,
                Jsonb(&shuffled_shoe(ranked_rules().decks)) as _,
                START_BALANCE
            )
            .execute(&mut *db)
            .await?;
            id
        }
    };
    let mut account = lock_account(db, user_id, id)
        .await?
        .ok_or_else(|| ApiError::internal("cuenta ranked recién creada no encontrada"))?;
    account.refill_if_due(OffsetDateTime::now_utc());
    Ok(account)
}

pub fn refill_pending(account: &Account) -> ApiError {
    let when = account
        .refill_available_at(OffsetDateTime::now_utc())
        .map(|t| format!(" (disponible el {})", rfc3339(t)))
        .unwrap_or_default();
    ApiError::new(
        StatusCode::CONFLICT,
        "refill_pending",
        format!("no te alcanza para la apuesta mínima; espera la recarga{when}"),
    )
}

/// `GET /ranked/me`: la cuenta de la temporada actual (la crea si no existe).
pub async fn me(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<RankedAccountView>> {
    let mut tx = state.pool.begin().await?;
    lock_user(&mut tx, user.id).await?;
    let mut account = lock_current_account(&mut tx, user.id).await?;
    account.save(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(account.view()))
}

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    season: Option<String>,
}

fn medal(s: Option<&str>) -> Option<Medal> {
    match s? {
        "gold" => Some(Medal::Gold),
        "silver" => Some(Medal::Silver),
        "bronze" => Some(Medal::Bronze),
        "top10" => Some(Medal::Top10),
        _ => None,
    }
}

/// `GET /ranked/leaderboard?season=AAAA-MM`: los 100 primeros por ganancia neta entre
/// quienes tienen el mínimo de manos, más la fila del usuario (provisional si no lo
/// tiene). Sin `season`, la actual. Es público: sin sesión no viene `me`.
pub async fn leaderboard(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(q): Query<LeaderboardQuery>,
) -> ApiResult<Json<RankedLeaderboard>> {
    let me = current_session(&jar, &state).await?.map(|s| s.user_id);
    let mut db = state.pool.acquire().await?;
    let season = match q.season {
        None => current_season(&mut db).await?,
        Some(id) => find_season(&mut db, &id)
            .await?
            .ok_or_else(|| ApiError::not_found("temporada no encontrada"))?,
    };
    let rows = sqlx::query!(
        r#"WITH q AS (
               SELECT a.user_id, a.net, a.hands, u.display_name,
                      rank() OVER (ORDER BY a.net DESC) AS pos,
                      row_number() OVER (ORDER BY a.net DESC, u.display_name_key) AS n
               FROM ranked_accounts a JOIN users u ON u.id = a.user_id
               WHERE a.season_id = $1 AND a.hands >= $2
           )
           SELECT q.user_id, q.net AS "net!", q.hands, q.display_name, q.pos AS "pos!", r.medal,
                  c.title AS "title?"
           FROM q LEFT JOIN season_results r ON r.season_id = $1 AND r.user_id = q.user_id
           LEFT JOIN user_cosmetics c ON c.user_id = q.user_id
           WHERE q.n <= 100
           ORDER BY q.n"#,
        season.id,
        MIN_HANDS
    )
    .fetch_all(&mut *db)
    .await?;
    let top = rows
        .into_iter()
        .map(|r| RankedRow {
            rank: r.pos as u32,
            display_name: r.display_name,
            title: title(r.title.as_deref()),
            net: r.net,
            hands: r.hands.max(0) as u32,
            provisional: false,
            medal: medal(r.medal.as_deref()),
            is_me: Some(r.user_id) == me,
        })
        .collect();
    let mine = match me {
        None => None,
        Some(user_id) => sqlx::query!(
            r#"SELECT a.net AS "net!", a.hands, u.display_name, r.medal, c.title AS "title?",
                      1 + (SELECT count(*) FROM ranked_accounts o
                           WHERE o.season_id = a.season_id AND o.hands >= $3 AND o.net > a.net)
                          AS "pos!"
               FROM ranked_accounts a
               JOIN users u ON u.id = a.user_id
               LEFT JOIN season_results r ON r.season_id = a.season_id AND r.user_id = a.user_id
               LEFT JOIN user_cosmetics c ON c.user_id = a.user_id
               WHERE a.season_id = $1 AND a.user_id = $2"#,
            season.id,
            user_id,
            MIN_HANDS
        )
        .fetch_optional(&mut *db)
        .await?
        .map(|r| RankedRow {
            rank: r.pos as u32,
            display_name: r.display_name,
            title: title(r.title.as_deref()),
            net: r.net,
            hands: r.hands.max(0) as u32,
            provisional: r.hands < MIN_HANDS,
            medal: medal(r.medal.as_deref()),
            is_me: true,
        }),
    };
    Ok(Json(RankedLeaderboard {
        season: season.id,
        starts_at: rfc3339(season.starts_at),
        ends_at: rfc3339(season.ends_at),
        closed: season.closed_at.is_some(),
        min_hands: MIN_HANDS as u32,
        top,
        me: mine,
    }))
}

/// Resultados de Ranked para `/me/records`.
pub async fn records(db: &mut PgConnection, user_id: Uuid) -> ApiResult<RankedRecords> {
    let seasons: Vec<SeasonResult> = sqlx::query!(
        "SELECT season_id, rank, net, hands, medal FROM season_results
         WHERE user_id = $1 ORDER BY season_id DESC",
        user_id
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| SeasonResult {
        season: r.season_id,
        rank: r.rank.max(0) as u32,
        net: r.net,
        hands: r.hands.max(0) as u32,
        medal: medal(r.medal.as_deref()),
    })
    .collect();
    Ok(RankedRecords {
        best_rank: seasons.iter().map(|s| s.rank).min(),
        seasons,
    })
}

/// Cierra las temporadas terminadas sin rondas en juego: guarda la posición final de
/// quienes tienen el mínimo de manos y asigna medallas (1.º, 2.º, 3.º y top 10 %).
/// Devuelve cuántas cerró. Una ronda abandonada se cierra sola a las 24 h
/// ([`super::play::close_stale_rounds`]), así que la espera es acotada.
pub async fn close_seasons(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let due = sqlx::query_scalar!(
        "SELECT id FROM seasons s
         WHERE ends_at <= now() AND closed_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM rounds r JOIN ranked_accounts a ON a.id = r.context_id
               WHERE r.kind = 'ranked' AND r.status = 'active' AND a.season_id = s.id
           )"
    )
    .fetch_all(pool)
    .await?;
    let mut closed = 0;
    for id in due {
        let mut tx = pool.begin().await?;
        let open = sqlx::query_scalar!(
            "SELECT closed_at IS NULL AS \"open!\" FROM seasons WHERE id = $1 FOR UPDATE",
            id
        )
        .fetch_one(&mut *tx)
        .await?;
        if !open {
            continue;
        }
        sqlx::query!(
            "INSERT INTO season_results (season_id, user_id, rank, net, hands, medal)
             SELECT season_id, user_id, pos, net, hands,
                    CASE WHEN pos = 1 THEN 'gold'
                         WHEN pos = 2 THEN 'silver'
                         WHEN pos = 3 THEN 'bronze'
                         WHEN pos <= ceil(total * 0.1) THEN 'top10'
                    END
             FROM (SELECT season_id, user_id, net, hands,
                          rank() OVER (ORDER BY net DESC) AS pos,
                          count(*) OVER () AS total
                   FROM ranked_accounts WHERE season_id = $1 AND hands >= $2) q",
            id,
            MIN_HANDS
        )
        .execute(&mut *tx)
        .await?;
        // Logro `rankedMedal` para quien recibió medalla.
        sqlx::query!(
            "INSERT INTO user_achievements (user_id, achievement)
             SELECT user_id, 'rankedMedal' FROM season_results
             WHERE season_id = $1 AND medal IS NOT NULL
             ON CONFLICT DO NOTHING",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("UPDATE seasons SET closed_at = now() WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        closed += 1;
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn max_bet_is_a_quarter_of_the_balance() {
        assert_eq!(max_bet(9), 0);
        assert_eq!(max_bet(10), 10);
        assert_eq!(max_bet(30), 10);
        assert_eq!(max_bet(100), 20);
        assert_eq!(max_bet(1_000), 250);
        assert_eq!(max_bet(4_000), 500);
        assert_eq!(max_bet(50_000), 500);
    }

    #[test]
    fn seasons_are_calendar_months_in_utc() {
        let (id, start, end) = season_bounds(datetime!(2026-10-28 15:30 UTC));
        assert_eq!(id, "2026-10");
        assert_eq!(start, datetime!(2026-10-01 00:00 UTC));
        assert_eq!(end, datetime!(2026-11-01 00:00 UTC));
        let (id, _, end) = season_bounds(datetime!(2026-12-31 23:59 UTC));
        assert_eq!(id, "2026-12");
        assert_eq!(end, datetime!(2027-01-01 00:00 UTC));
        // 23:30 del 31 en México ya es día 1 en UTC.
        let (id, ..) = season_bounds(datetime!(2026-10-31 23:30 -6));
        assert_eq!(id, "2026-11");
    }

    fn account(balance: i64, last_refill_at: Option<OffsetDateTime>) -> Account {
        Account {
            id: Uuid::nil(),
            season_id: "2026-10".into(),
            season_ends_at: datetime!(2026-11-01 00:00 UTC),
            rules: ranked_rules(),
            shoe: GameShoe::ordered(6),
            balance,
            peak: START_BALANCE,
            hands: 0,
            refilled_total: 0,
            last_refill_at,
            refill: None,
        }
    }

    #[test]
    fn refills_to_the_floor_at_most_once_a_day() {
        let now = datetime!(2026-10-28 12:00 UTC);
        let mut a = account(30, None);
        assert_eq!(a.refill_if_due(now), Some(70));
        assert_eq!((a.balance, a.refilled_total, a.net()), (100, 70, -470));

        a.balance = 40;
        let later = now + Duration::hours(23);
        assert_eq!(a.refill_if_due(later), None);
        assert_eq!(a.refill_available_at(later), Some(now + REFILL_EVERY));
        assert_eq!(a.refill_if_due(now + REFILL_EVERY), Some(60));
        assert_eq!(a.refilled_total, 130);

        let mut rich = account(150, Some(now));
        assert_eq!(rich.refill_if_due(now), None);
        assert_eq!(
            rich.refill_available_at(now),
            None,
            "sin aviso si no está bajo el piso"
        );
    }
}
