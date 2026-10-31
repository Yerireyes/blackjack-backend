//! Logros: se otorgan en el servidor al liquidar cada ronda, en la misma transacción
//! (así la respuesta guardada para la idempotencia ya los trae), y al cerrar una
//! temporada ranked. `GET /me/achievements` los lista con la racha de decisiones
//! óptimas.

use axum::extract::State;
use axum::Json;
use engine::game::{
    AchievementId, HandOutcome, MyAchievements, Round, RoundKind, UnlockedAchievement,
};
use engine::Rules;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::routes::casual::rfc3339;
use crate::routes::ranked::MIN_HANDS;
use crate::state::AppState;

/// Manos de una partida casual a 6:5 para `SixToFiveSurvivor`.
const SIX_TO_FIVE_HANDS: i32 = 50;
/// Saldo de una partida casual para `DoubleUp`.
const DOUBLE_UP_BALANCE: i64 = 1_000;

/// Lo que importa de la cartera después de liquidar.
pub enum WalletFacts<'a> {
    Casual {
        rules: &'a Rules,
        hands: i32,
        peak: i64,
    },
    Ranked {
        hands: i32,
    },
    Daily {
        /// Jugó las 30 manos (no se quedó sin saldo antes).
        completed: bool,
    },
}

/// Una ronda recién liquidada.
pub struct Settlement<'a> {
    pub kind: RoundKind,
    pub round: &'a Round,
    pub net: i64,
    pub wallet: WalletFacts<'a>,
}

/// Racha de decisiones óptimas tras sumar las de esta ronda: sigue mientras se elija
/// la óptima y vuelve a 0 con la primera que no.
pub fn update_streak(mut streak: i32, round: &Round) -> (i32, i32) {
    let mut best = streak;
    for d in &round.decisions {
        streak = if d.chosen == d.best { streak + 1 } else { 0 };
        best = best.max(streak);
    }
    (streak, best)
}

/// Logros que cumple esta liquidación (sin mirar si ya los tenía). `streak` es la
/// racha de decisiones óptimas más alta alcanzada durante la ronda.
pub fn earned(s: &Settlement, streak: i32) -> Vec<AchievementId> {
    let mut out = Vec::new();
    let hands = &s.round.hands;
    if s.net > 0 {
        out.push(AchievementId::FirstWin);
    }
    if hands
        .iter()
        .any(|h| h.outcome == Some(HandOutcome::Blackjack))
    {
        out.push(AchievementId::Blackjack);
    }
    if hands.len() >= 2 && hands.iter().all(|h| h.outcome == Some(HandOutcome::Win)) {
        out.push(AchievementId::PerfectSplit);
    }
    if streak >= 10 {
        out.push(AchievementId::Optimal10);
    }
    if streak >= 50 {
        out.push(AchievementId::Optimal50);
    }
    match s.wallet {
        WalletFacts::Casual { rules, hands, peak } => {
            if rules.blackjack_pays < 1.5 && hands >= SIX_TO_FIVE_HANDS {
                out.push(AchievementId::SixToFiveSurvivor);
            }
            if peak >= DOUBLE_UP_BALANCE {
                out.push(AchievementId::DoubleUp);
            }
        }
        WalletFacts::Ranked { hands } => {
            if hands >= MIN_HANDS {
                out.push(AchievementId::RankedQualified);
            }
        }
        WalletFacts::Daily { completed } => {
            if completed {
                out.push(AchievementId::DailyComplete);
            }
        }
    }
    out
}

/// Guarda los logros nuevos de esta liquidación (y la racha, en los modos con
/// revisión) y devuelve los que el usuario no tenía.
pub async fn on_settle(
    db: &mut PgConnection,
    user_id: Uuid,
    s: &Settlement<'_>,
) -> ApiResult<Vec<AchievementId>> {
    let mut best_in_round = 0;
    if s.kind != RoundKind::Casual && !s.round.decisions.is_empty() {
        let current = sqlx::query_scalar!(
            "SELECT optimal_streak FROM player_stats WHERE user_id = $1",
            user_id
        )
        .fetch_optional(&mut *db)
        .await?
        .unwrap_or(0);
        let (streak, best) = update_streak(current, s.round);
        best_in_round = best;
        sqlx::query!(
            "INSERT INTO player_stats (user_id, optimal_streak, best_optimal_streak)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id) DO UPDATE
             SET optimal_streak = $2,
                 best_optimal_streak = greatest(player_stats.best_optimal_streak, $3)",
            user_id,
            streak,
            best
        )
        .execute(&mut *db)
        .await?;
    }
    let ids: Vec<String> = earned(s, best_in_round)
        .into_iter()
        .map(|a| a.as_str().to_string())
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let new = sqlx::query_scalar!(
        "INSERT INTO user_achievements (user_id, achievement)
         SELECT $1, unnest($2::text[])
         ON CONFLICT DO NOTHING
         RETURNING achievement",
        user_id,
        &ids
    )
    .fetch_all(db)
    .await?;
    // En el orden del catálogo, no en el que devuelve la base.
    Ok(AchievementId::ALL
        .into_iter()
        .filter(|a| new.iter().any(|n| n == a.as_str()))
        .collect())
}

/// `GET /me/achievements`.
pub async fn mine(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<MyAchievements>> {
    let mut db = state.pool.acquire().await?;
    let rows = sqlx::query!(
        "SELECT achievement, unlocked_at FROM user_achievements
         WHERE user_id = $1 ORDER BY unlocked_at, achievement",
        user.id
    )
    .fetch_all(&mut *db)
    .await?;
    let unlocked = rows
        .into_iter()
        .map(|r| {
            let id = AchievementId::parse(&r.achievement).ok_or_else(|| {
                ApiError::internal(format!("logro desconocido: {}", r.achievement))
            })?;
            Ok(UnlockedAchievement {
                id,
                unlocked_at: rfc3339(r.unlocked_at),
            })
        })
        .collect::<ApiResult<_>>()?;
    let stats = sqlx::query!(
        "SELECT optimal_streak, best_optimal_streak FROM player_stats WHERE user_id = $1",
        user.id
    )
    .fetch_optional(&mut *db)
    .await?;
    let (streak, best) = stats.map_or((0, 0), |s| (s.optimal_streak, s.best_optimal_streak));
    Ok(Json(MyAchievements {
        unlocked,
        optimal_streak: streak.max(0) as u32,
        best_optimal_streak: best.max(0) as u32,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::game::{GameShoe, Preset, ReviewItem};
    use engine::Action;

    fn round(top: &[engine::game::Face]) -> Round {
        let rules = Preset::Vegas.rules();
        let mut shoe = GameShoe::stacked(6, top);
        Round::deal(&mut shoe, &rules).unwrap()
    }

    fn decision(chosen: Action, best: Action) -> ReviewItem {
        ReviewItem {
            hand_index: 0,
            chosen,
            best,
            ev_chosen: 0.0,
            ev_best: 0.0,
            exact: true,
            spot: None,
        }
    }

    #[test]
    fn streak_resets_on_a_mistake_and_keeps_the_best() {
        let mut r = round(&[engine::game::Face::Ten; 4]);
        r.decisions = vec![
            decision(Action::Stand, Action::Stand),
            decision(Action::Hit, Action::Hit),
            decision(Action::Hit, Action::Stand),
            decision(Action::Stand, Action::Stand),
        ];
        assert_eq!(update_streak(8, &r), (1, 10));
        assert_eq!(
            update_streak(0, &round(&[engine::game::Face::Ten; 4])),
            (0, 0)
        );
    }

    #[test]
    fn earned_by_outcome_and_wallet() {
        use engine::game::Face::*;
        // Blackjack del jugador contra un 9: se liquida en el reparto y gana.
        let bj = round(&[Ace, Nine, K, Seven]);
        let vegas = Preset::Vegas.rules();
        let six_five = Preset::SixToFive.rules();
        let s = Settlement {
            kind: RoundKind::Casual,
            round: &bj,
            net: 15,
            wallet: WalletFacts::Casual {
                rules: &six_five,
                hands: 50,
                peak: 1_000,
            },
        };
        assert_eq!(
            earned(&s, 0),
            [
                AchievementId::FirstWin,
                AchievementId::Blackjack,
                AchievementId::SixToFiveSurvivor,
                AchievementId::DoubleUp
            ]
        );
        let s = Settlement {
            kind: RoundKind::Ranked,
            round: &bj,
            net: 15,
            wallet: WalletFacts::Ranked { hands: 199 },
        };
        assert_eq!(
            earned(&s, 10),
            [
                AchievementId::FirstWin,
                AchievementId::Blackjack,
                AchievementId::Optimal10
            ]
        );
        let s = Settlement {
            kind: RoundKind::Casual,
            round: &bj,
            net: 0,
            wallet: WalletFacts::Casual {
                rules: &vegas,
                hands: 80,
                peak: 999,
            },
        };
        // 3:2 no cuenta para el de 6:5.
        assert_eq!(earned(&s, 0), [AchievementId::Blackjack]);
        let s = Settlement {
            kind: RoundKind::Daily,
            round: &bj,
            net: -10,
            wallet: WalletFacts::Daily { completed: true },
        };
        assert!(earned(&s, 50).contains(&AchievementId::Optimal50));
        assert!(earned(&s, 50).contains(&AchievementId::DailyComplete));
    }
}
