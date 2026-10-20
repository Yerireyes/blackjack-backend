//! Contrato con el front. Mismos structs para WASM y HTTP; JSON en camelCase.

use serde::{Deserialize, Serialize};

use crate::dealer::{DealerDist, BLACKJACK, BUST};
use crate::rank::Rank;
use crate::rules::Rules;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum Action {
    Stand,
    Hit,
    Double,
    Split,
    Surrender,
}

/// Qué acciones permite la situación de la mesa (el front puede restringir más).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct Allowed {
    pub double: bool,
    pub split: bool,
    pub surrender: bool,
}

impl Default for Allowed {
    fn default() -> Allowed {
        Allowed {
            double: true,
            split: true,
            surrender: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeHandRequest {
    pub version: u32,
    pub rules: Rules,
    /// Cartas que quedan sin ver, por rango. Incluye la carta oculta del dealer
    /// y excluye las cartas visibles (mano del jugador, carta visible del dealer...).
    pub shoe: [u16; 10],
    pub hand: Vec<Rank>,
    pub dealer_up: Rank,
    /// Cuántas veces se ha dividido ya en esta ronda (0 = la mano no viene de dividir).
    #[serde(default)]
    pub split_hands_so_far: u8,
    /// La mano viene de dividir ases.
    #[serde(default)]
    pub is_split_aces: bool,
    #[serde(default)]
    pub allowed: Allowed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct ActionResult {
    pub action: Action,
    pub ev: f64,
    pub p_win: f64,
    pub p_push: f64,
    pub p_lose: f64,
    pub exact: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct DealerDistribution {
    pub p17: f64,
    pub p18: f64,
    pub p19: f64,
    pub p20: f64,
    pub p21: f64,
    pub p_blackjack: f64,
    pub p_bust: f64,
}

impl From<DealerDist> for DealerDistribution {
    fn from(d: DealerDist) -> Self {
        DealerDistribution {
            p17: d.total(17),
            p18: d.total(18),
            p19: d.total(19),
            p20: d.total(20),
            p21: d.total(21),
            p_blackjack: d.p[BLACKJACK],
            p_bust: d.p[BUST],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeHandResponse {
    pub version: u32,
    pub actions: Vec<ActionResult>,
    pub best: Action,
    pub p_bust_on_hit: f64,
    pub dealer: DealerDistribution,
    /// Lo mide el binding (WASM/HTTP); el motor no usa reloj y lo deja en 0.
    pub compute_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeDealerRequest {
    pub version: u32,
    pub rules: Rules,
    /// Cartas sin ver. Si `hole` viene informada, NO debe incluirla.
    pub shoe: [u16; 10],
    pub up: Rank,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub hole: Option<Rank>,
    /// Cartas que el dealer ya sacó después de la visible y la oculta, en orden.
    /// Requiere `hole`; el zapato tampoco las incluye.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub drawn: Vec<Rank>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeDealerResponse {
    pub version: u32,
    pub dealer: DealerDistribution,
    /// Probabilidad de que el dealer se pase con la próxima carta. Solo viene cuando
    /// se conocen todas sus cartas (`hole` informada) y las reglas lo obligan a pedir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub p_bust_next: Option<f64>,
    pub compute_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeRoundRequest {
    pub version: u32,
    pub rules: Rules,
    /// Cartas sin ver: incluye la oculta y excluye todas las manos visibles.
    pub shoe: [u16; 10],
    pub up: Rank,
    pub hands: Vec<Vec<Rank>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AnalyzeRoundResponse {
    pub version: u32,
    pub hands: Vec<AnalyzeHandResponse>,
    /// −promedio del mejor EV de cada mano. Aproximación: ignora las cartas que
    /// sacarán los otros jugadores después.
    pub house_edge_round: f64,
    pub compute_ms: f64,
}
