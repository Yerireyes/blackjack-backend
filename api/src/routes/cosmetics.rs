//! Cosméticos: paño, dorso, fichas y título. Solo cambian cómo se ve la mesa; nunca dan
//! fichas ni ventaja. Lo que el usuario tiene sale de sus logros (más los de serie), así
//! que no se guarda: solo se guarda lo que tiene puesto.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use engine::game::{
    AchievementId, CosmeticId, CosmeticItem, CosmeticSlot, EquippedCosmetics, MyCosmetics,
};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::extract::ApiJson;
use crate::state::AppState;

use AchievementId as A;
use CosmeticId as C;
use CosmeticSlot as S;

/// El catálogo: cada cosmético, su lugar y el logro que lo desbloquea (`None`: de serie).
pub const CATALOG: [(CosmeticId, CosmeticSlot, Option<AchievementId>); 14] = [
    (C::FeltGreen, S::Felt, None),
    (C::FeltBlue, S::Felt, Some(A::DailyComplete)),
    (C::FeltRed, S::Felt, Some(A::DoubleUp)),
    (C::FeltBlack, S::Felt, Some(A::RankedMedal)),
    (C::BackClassic, S::CardBack, None),
    (C::BackGold, S::CardBack, Some(A::Blackjack)),
    (C::BackMidnight, S::CardBack, Some(A::PerfectSplit)),
    (C::ChipsClassic, S::Chips, None),
    (C::ChipsNeon, S::Chips, Some(A::SixToFiveSurvivor)),
    (C::ChipsGold, S::Chips, Some(A::RankedQualified)),
    (C::TitleRookie, S::Title, Some(A::FirstWin)),
    (C::TitleCoolHead, S::Title, Some(A::Optimal10)),
    (C::TitleByTheBook, S::Title, Some(A::Optimal50)),
    (C::TitleMedalist, S::Title, Some(A::RankedMedal)),
];

/// Lo que tiene puesto quien nunca eligió.
pub fn default_equipped() -> EquippedCosmetics {
    EquippedCosmetics {
        felt: C::FeltGreen,
        card_back: C::BackClassic,
        chips: C::ChipsClassic,
        title: None,
    }
}

fn slot_of(id: CosmeticId) -> CosmeticSlot {
    CATALOG
        .iter()
        .find(|(c, ..)| *c == id)
        .map(|(_, s, _)| *s)
        .expect("todo cosmético está en el catálogo")
}

/// Los cosméticos que dan estos logros, más los de serie, en el orden del catálogo.
pub fn owned(achievements: &[AchievementId]) -> Vec<CosmeticId> {
    CATALOG
        .iter()
        .filter(|(_, _, by)| by.is_none_or(|a| achievements.contains(&a)))
        .map(|(c, ..)| *c)
        .collect()
}

/// Nombre estable (el mismo del JSON), para guardarlo en la base.
fn as_str(id: CosmeticId) -> String {
    match serde_json::to_value(id) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("CosmeticId se serializa como texto"),
    }
}

fn parse(s: &str) -> ApiResult<CosmeticId> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|_| ApiError::internal(format!("cosmético desconocido: {s}")))
}

async fn achievements_of(db: &mut PgConnection, user_id: Uuid) -> ApiResult<Vec<AchievementId>> {
    let rows = sqlx::query_scalar!(
        "SELECT achievement FROM user_achievements WHERE user_id = $1",
        user_id
    )
    .fetch_all(db)
    .await?;
    // Un id que ya no está en el catálogo no da nada.
    Ok(rows
        .iter()
        .filter_map(|a| AchievementId::parse(a))
        .collect())
}

async fn load(db: &mut PgConnection, user_id: Uuid) -> ApiResult<MyCosmetics> {
    let owned = owned(&achievements_of(db, user_id).await?);
    let row = sqlx::query!(
        "SELECT felt, card_back, chips, title FROM user_cosmetics WHERE user_id = $1",
        user_id
    )
    .fetch_optional(db)
    .await?;
    let equipped = match row {
        None => default_equipped(),
        Some(r) => EquippedCosmetics {
            felt: parse(&r.felt)?,
            card_back: parse(&r.card_back)?,
            chips: parse(&r.chips)?,
            title: r.title.as_deref().map(parse).transpose()?,
        },
    };
    Ok(MyCosmetics { owned, equipped })
}

/// `GET /cosmetics`: el catálogo (público).
pub async fn catalog() -> Json<Vec<CosmeticItem>> {
    Json(
        CATALOG
            .iter()
            .map(|&(id, slot, unlocked_by)| CosmeticItem {
                id,
                slot,
                unlocked_by,
            })
            .collect(),
    )
}

/// `GET /me/cosmetics`.
pub async fn mine(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<MyCosmetics>> {
    let mut db = state.pool.acquire().await?;
    Ok(Json(load(&mut db, user.id).await?))
}

/// `PUT /me/cosmetics`: cambia lo que tiene puesto (todo a la vez). Cada uno tiene que
/// ir en su lugar y estar desbloqueado.
pub async fn equip(
    State(state): State<AppState>,
    user: AuthUser,
    ApiJson(req): ApiJson<EquippedCosmetics>,
) -> ApiResult<Json<MyCosmetics>> {
    let mut db = state.pool.acquire().await?;
    let owned = owned(&achievements_of(&mut db, user.id).await?);
    let picks = [
        (Some(req.felt), S::Felt, "felt"),
        (Some(req.card_back), S::CardBack, "cardBack"),
        (Some(req.chips), S::Chips, "chips"),
        (req.title, S::Title, "title"),
    ];
    for (pick, slot, field) in picks {
        let Some(id) = pick else { continue };
        if slot_of(id) != slot {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "cosmetic_wrong_slot",
                format!("{} no va en {field}", as_str(id)),
            ));
        }
        if !owned.contains(&id) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "cosmetic_locked",
                format!("todavía no desbloqueaste {}", as_str(id)),
            ));
        }
    }
    sqlx::query!(
        "INSERT INTO user_cosmetics (user_id, felt, card_back, chips, title)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id) DO UPDATE
         SET felt = $2, card_back = $3, chips = $4, title = $5, updated_at = now()",
        user.id,
        as_str(req.felt),
        as_str(req.card_back),
        as_str(req.chips),
        req.title.map(as_str)
    )
    .execute(&mut *db)
    .await?;
    Ok(Json(MyCosmetics {
        owned,
        equipped: req,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_achievement_unlocks_something_and_ids_are_unique() {
        for a in AchievementId::ALL {
            assert!(
                CATALOG.iter().any(|(_, _, by)| *by == Some(a)),
                "{a:?} no desbloquea nada"
            );
        }
        for (i, (c, ..)) in CATALOG.iter().enumerate() {
            assert!(!CATALOG[..i].iter().any(|(d, ..)| d == c), "{c:?} repetido");
            assert_eq!(parse(&as_str(*c)).unwrap(), *c);
        }
    }

    #[test]
    fn owned_are_the_defaults_plus_the_achievements() {
        assert_eq!(owned(&[]), [C::FeltGreen, C::BackClassic, C::ChipsClassic]);
        assert_eq!(
            owned(&[A::RankedMedal]),
            [
                C::FeltGreen,
                C::FeltBlack,
                C::BackClassic,
                C::ChipsClassic,
                C::TitleMedalist
            ]
        );
        let defaults = default_equipped();
        for id in [defaults.felt, defaults.card_back, defaults.chips] {
            assert!(owned(&[]).contains(&id));
        }
    }
}
