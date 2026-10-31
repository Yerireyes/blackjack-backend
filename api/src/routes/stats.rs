//! `GET /me/history` y `GET /me/stats`: las rondas liquidadas del usuario y lo que dicen
//! de su juego. Las estadísticas de decisiones salen de la revisión de Ranked y del reto
//! diario (Casual no se revisa) y señalan las situaciones donde más EV se pierde, para el
//! entrenamiento dirigido del front.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::Json;
use engine::game::{
    DecisionSpot, DecisionStats, HistoryItem, HistoryPage, KindStats, ModeStats, MyStats,
    ReviewItem, Round, RoundKind, SpotKind, SpotStats,
};
use serde::Deserialize;
use sqlx::types::Json as Jsonb;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::routes::casual::rfc3339;
use crate::state::AppState;

const DEFAULT_PAGE: i64 = 20;
const MAX_PAGE: i64 = 50;
/// Rondas revisadas que entran en las estadísticas de decisiones (las más recientes).
const MAX_REVIEWED_ROUNDS: i64 = 5_000;
const WEAK_SPOTS: usize = 10;

fn kind_of(s: &str) -> ApiResult<RoundKind> {
    match s {
        "casual" => Ok(RoundKind::Casual),
        "ranked" => Ok(RoundKind::Ranked),
        "daily" => Ok(RoundKind::Daily),
        other => Err(ApiError::internal(format!("modo desconocido: {other}"))),
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    kind: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
}

/// `GET /me/history?kind=&before=&limit=`: rondas liquidadas, de la más reciente a la
/// más antigua, de a `limit` (1–50, 20 por defecto). `before` es el `nextBefore` de la
/// página anterior.
pub async fn history(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<HistoryPage>> {
    if let Some(k) = q.kind.as_deref() {
        if !matches!(k, "casual" | "ranked" | "daily") {
            return Err(ApiError::bad_request(
                "kind debe ser casual, ranked o daily",
            ));
        }
    }
    let before = q
        .before
        .as_deref()
        .map(|b| {
            OffsetDateTime::parse(b, &Rfc3339)
                .map_err(|_| ApiError::bad_request("before debe ser una fecha RFC 3339"))
        })
        .transpose()?;
    let limit = q.limit.unwrap_or(DEFAULT_PAGE);
    if !(1..=MAX_PAGE).contains(&limit) {
        return Err(ApiError::bad_request(format!("limit va de 1 a {MAX_PAGE}")));
    }
    let rows = sqlx::query!(
        r#"SELECT id, kind, context_id, bet, net AS "net!", state AS "state: Jsonb<Round>",
                  settled_at AS "settled_at!"
           FROM rounds
           WHERE user_id = $1 AND status = 'settled'
             AND ($2::text IS NULL OR kind = $2)
             AND ($3::timestamptz IS NULL OR settled_at < $3)
           ORDER BY settled_at DESC, id DESC
           LIMIT $4"#,
        user.id,
        q.kind,
        before,
        limit + 1
    )
    .fetch_all(&state.pool)
    .await?;
    let more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.into_iter().take(limit as usize) {
        let kind = kind_of(&r.kind)?;
        let round = r.state.0;
        items.push(HistoryItem {
            id: r.id.to_string(),
            kind,
            context_id: r.context_id.to_string(),
            bet: r.bet,
            net: r.net,
            settled_at: rfc3339(r.settled_at),
            player: round.player_view(r.bet),
            dealer: round.dealer_view(),
            peek_blackjack: round.peek_blackjack,
            insured: round.insurance,
            insurance_net: round.insurance_net(r.bet),
            review: (kind != RoundKind::Casual).then(|| round.decisions.clone()),
        });
    }
    let next_before = if more {
        items.last().map(|i| i.settled_at.clone())
    } else {
        None
    };
    Ok(Json(HistoryPage { items, next_before }))
}

/// Agrega las decisiones revisadas: total, óptimas, EV perdido, por tipo de mano y las
/// situaciones donde más se pierde.
pub fn decision_stats<'a>(decisions: impl IntoIterator<Item = &'a ReviewItem>) -> DecisionStats {
    let mut out = DecisionStats::default();
    let mut kinds: HashMap<SpotKind, KindStats> = HashMap::new();
    let mut spots: HashMap<DecisionSpot, SpotStats> = HashMap::new();
    for d in decisions {
        let optimal = d.chosen == d.best;
        let lost = (d.ev_best - d.ev_chosen).max(0.0);
        out.decisions += 1;
        out.optimal += optimal as u32;
        out.ev_lost += lost;
        let Some(spot) = d.spot else { continue };
        let k = kinds.entry(spot.kind).or_insert(KindStats {
            kind: spot.kind,
            decisions: 0,
            optimal: 0,
            ev_lost: 0.0,
        });
        k.decisions += 1;
        k.optimal += optimal as u32;
        k.ev_lost += lost;
        let s = spots.entry(spot).or_insert(SpotStats {
            spot,
            decisions: 0,
            optimal: 0,
            ev_lost: 0.0,
        });
        s.decisions += 1;
        s.optimal += optimal as u32;
        s.ev_lost += lost;
    }
    out.by_kind = [SpotKind::Hard, SpotKind::Soft, SpotKind::Pair]
        .into_iter()
        .filter_map(|k| kinds.remove(&k))
        .collect();
    let mut weak: Vec<SpotStats> = spots
        .into_values()
        .filter(|s| s.optimal < s.decisions)
        .collect();
    // Más EV perdido primero; en empate, más errores y después un orden fijo.
    weak.sort_by(|a, b| {
        b.ev_lost
            .total_cmp(&a.ev_lost)
            .then((b.decisions - b.optimal).cmp(&(a.decisions - a.optimal)))
            .then(a.spot.cmp(&b.spot))
    });
    weak.truncate(WEAK_SPOTS);
    out.weak_spots = weak;
    out
}

async fn mode_stats(state: &AppState, user_id: Uuid) -> ApiResult<Vec<ModeStats>> {
    let rows = sqlx::query!(
        r#"SELECT kind,
                  count(*) AS "rounds!",
                  coalesce(sum(net), 0)::bigint AS "net!",
                  count(*) FILTER (WHERE net > 0) AS "wins!",
                  count(*) FILTER (WHERE net = 0) AS "pushes!",
                  count(*) FILTER (WHERE net < 0) AS "losses!",
                  count(*) FILTER (
                      WHERE jsonb_path_exists(state, '$.hands[*] ? (@.outcome == "blackjack")')
                  ) AS "blackjacks!",
                  coalesce(max(net), 0) AS "best!",
                  coalesce(min(net), 0) AS "worst!"
           FROM rounds WHERE user_id = $1 AND status = 'settled'
           GROUP BY kind"#,
        user_id
    )
    .fetch_all(&state.pool)
    .await?;
    let mut modes = rows
        .into_iter()
        .map(|r| {
            Ok(ModeStats {
                kind: kind_of(&r.kind)?,
                rounds: r.rounds as u32,
                net: r.net,
                wins: r.wins as u32,
                pushes: r.pushes as u32,
                losses: r.losses as u32,
                blackjacks: r.blackjacks as u32,
                best_win: r.best.max(0),
                worst_loss: r.worst.min(0),
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let order = |k: RoundKind| match k {
        RoundKind::Casual => 0,
        RoundKind::Ranked => 1,
        RoundKind::Daily => 2,
    };
    modes.sort_by_key(|m| order(m.kind));
    Ok(modes)
}

/// `GET /me/stats`.
pub async fn stats(State(state): State<AppState>, user: AuthUser) -> ApiResult<Json<MyStats>> {
    let modes = mode_stats(&state, user.id).await?;
    let reviewed = sqlx::query_scalar!(
        r#"SELECT state->'decisions' AS "decisions!: Jsonb<Vec<ReviewItem>>"
           FROM rounds
           WHERE user_id = $1 AND status = 'settled' AND kind IN ('ranked', 'daily')
             AND jsonb_array_length(state->'decisions') > 0
           ORDER BY settled_at DESC
           LIMIT $2"#,
        user.id,
        MAX_REVIEWED_ROUNDS
    )
    .fetch_all(&state.pool)
    .await?;
    let decisions = decision_stats(reviewed.iter().flat_map(|d| d.0.iter()));
    Ok(Json(MyStats { modes, decisions }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Action;

    fn item(chosen: Action, best: Action, lost: f64, spot: Option<DecisionSpot>) -> ReviewItem {
        ReviewItem {
            hand_index: 0,
            chosen,
            best,
            ev_chosen: -lost,
            ev_best: 0.0,
            exact: true,
            spot,
        }
    }

    fn spot(kind: SpotKind, total: u8, up: u8) -> Option<DecisionSpot> {
        Some(DecisionSpot {
            kind,
            total,
            pair: None,
            dealer_up: up,
        })
    }

    #[test]
    fn groups_decisions_and_ranks_weak_spots_by_ev_lost() {
        use Action::*;
        let hard16 = spot(SpotKind::Hard, 16, 9);
        let soft18 = spot(SpotKind::Soft, 18, 8);
        let items = [
            item(Stand, Hit, 0.05, hard16),
            item(Stand, Hit, 0.05, hard16),
            item(Hit, Hit, 0.0, hard16),
            item(Stand, Hit, 0.2, soft18),
            item(Stand, Stand, 0.0, spot(SpotKind::Hard, 12, 3)),
            // Sin situación (rondas viejas): cuenta en el total, no en los grupos.
            item(Hit, Stand, 0.1, None),
        ];
        let s = decision_stats(&items);
        assert_eq!((s.decisions, s.optimal), (6, 2));
        assert!((s.ev_lost - 0.4).abs() < 1e-12);
        assert_eq!(
            s.by_kind
                .iter()
                .map(|k| (k.kind, k.decisions, k.optimal))
                .collect::<Vec<_>>(),
            [(SpotKind::Hard, 4, 2), (SpotKind::Soft, 1, 0)]
        );
        // Lo que no tuvo errores no es un punto débil.
        assert_eq!(s.weak_spots.len(), 2);
        assert_eq!(s.weak_spots[0].spot, soft18.unwrap());
        assert_eq!((s.weak_spots[1].decisions, s.weak_spots[1].optimal), (3, 1));
    }
}
