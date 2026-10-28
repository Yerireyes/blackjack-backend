//! Contrato HTTP de Casual (y más adelante Ranked) con el front. Se generan en
//! `contract.ts` con `npm run gen:contract`, como los de `/analyze/*`. JSON en camelCase.

use serde::{Deserialize, Serialize};

use super::card::{Card, Face, Suit};
use crate::dto::Action;
use crate::rules::{DoubleOn, Rules, Surrender};

/// Carta visible: figura y palo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct CardView {
    pub rank: Face,
    pub suit: Suit,
}

impl From<Card> for CardView {
    fn from(c: Card) -> CardView {
        CardView {
            rank: c.face,
            suit: c.suit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum HandOutcome {
    Blackjack,
    Win,
    Push,
    Lose,
    Bust,
    Surrender,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct HandView {
    pub cards: Vec<CardView>,
    pub doubled: bool,
    pub surrendered: bool,
    pub from_split: bool,
    /// Solo en una ronda liquidada.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub outcome: Option<HandOutcome>,
    /// Neto de esta mano en fichas (solo liquidada).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub net: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct PlayerView {
    pub hands: Vec<HandView>,
    /// Índice de la mano que se está jugando.
    pub active: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct DealerView {
    pub up: CardView,
    /// `null` mientras no se revela.
    #[cfg_attr(feature = "ts", tsify(type = "CardView | null"))]
    pub hole: Option<CardView>,
    /// Cartas que pidió el dealer, en orden, para animarlas una a una.
    pub hits: Vec<CardView>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct AllowedActions {
    pub hit: bool,
    pub stand: bool,
    pub double: bool,
    pub split: bool,
    pub surrender: bool,
}

impl AllowedActions {
    pub fn get(&self, action: Action) -> bool {
        match action {
            Action::Hit => self.hit,
            Action::Stand => self.stand,
            Action::Double => self.double,
            Action::Split => self.split,
            Action::Surrender => self.surrender,
        }
    }
}

/// El turno del dealer se resuelve dentro de la misma respuesta: no hay fase visible
/// para él.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum RoundPhase {
    Player,
    Settled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum RoundKind {
    Casual,
    Ranked,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct WalletView {
    pub balance: i64,
    /// Apuesta máxima para la próxima ronda (0 si el saldo no alcanza el mínimo).
    pub max_bet: i64,
    /// Ranked: cuándo se puede volver a recargar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub refill_available_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct ReviewItem {
    pub hand_index: u32,
    pub chosen: Action,
    pub best: Action,
    pub ev_chosen: f64,
    pub ev_best: f64,
    pub exact: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct RoundResult {
    /// Neto de la ronda en fichas.
    pub net: i64,
    /// Ranked: recarga aplicada en esta liquidación.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub refilled: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub review: Option<Vec<ReviewItem>>,
}

/// Lo único que el cliente ve de una ronda. Nunca incluye la oculta sin revelar ni
/// el orden del zapato.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct RoundView {
    pub id: String,
    pub kind: RoundKind,
    /// Id de la partida casual (o de la cuenta ranked) a la que pertenece.
    pub context_id: String,
    pub bet: i64,
    pub phase: RoundPhase,
    pub player: PlayerView,
    pub dealer: DealerView,
    pub allowed: AllowedActions,
    /// El dealer tenía blackjack al revisar la oculta: la ronda terminó en el reparto.
    pub peek_blackjack: bool,
    pub wallet: WalletView,
    /// Solo casual: composición restante por rango (0 = As … 9 = diez), con la
    /// oculta contada como no vista.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub shoe_counts: Option<[u16; 10]>,
    /// Fracción del zapato ya repartida.
    pub penetration: f64,
    /// Solo en `settled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub result: Option<RoundResult>,
}

/// `POST /play/rounds`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct NewRoundRequest {
    pub kind: RoundKind,
    /// Casual: id de la partida activa. Si falta, se usa la partida activa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub context_id: Option<String>,
    pub bet: i64,
    /// UUID generado por el cliente; repetirlo devuelve la misma respuesta.
    pub idempotency_key: String,
}

/// `POST /play/rounds/{id}/actions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct RoundActionRequest {
    pub action: Action,
    pub idempotency_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum Preset {
    Vegas,
    Europe,
    SixToFive,
}

impl Preset {
    /// Las mismas reglas que los presets del front.
    pub fn rules(self) -> Rules {
        let vegas = Rules {
            decks: 6,
            dealer_hits_soft17: false,
            double_after_split: true,
            double_on: DoubleOn::Any,
            max_split_hands: 4,
            resplit_aces: false,
            hit_split_aces: false,
            surrender: Surrender::Late,
            dealer_peeks: true,
            blackjack_pays: 1.5,
            penetration: 0.75,
        };
        match self {
            Preset::Vegas => vegas,
            Preset::Europe => Rules {
                double_on: DoubleOn::NineToEleven,
                max_split_hands: 2,
                surrender: Surrender::None,
                dealer_peeks: false,
                ..vegas
            },
            Preset::SixToFive => Rules {
                decks: 1,
                dealer_hits_soft17: true,
                double_after_split: false,
                max_split_hands: 2,
                surrender: Surrender::None,
                blackjack_pays: 1.2,
                penetration: 0.6,
                ..vegas
            },
        }
    }
}

/// `POST /casual/runs`: un preset o reglas personalizadas (exactamente uno).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct CreateCasualRunRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub preset: Option<Preset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub rules: Option<Rules>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum CasualRunStatus {
    Active,
    /// El usuario se retiró.
    Retired,
    /// El saldo quedó por debajo de la apuesta mínima.
    Busted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct CasualRunView {
    pub id: String,
    pub rules: Rules,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub preset: Option<Preset>,
    pub status: CasualRunStatus,
    pub balance: i64,
    /// Saldo máximo alcanzado.
    pub peak: i64,
    /// Manos (rondas) liquidadas.
    pub hands: u32,
    /// Apuesta máxima para la próxima ronda (0 si la partida terminó).
    pub max_bet: i64,
    /// Fracción del zapato ya repartida.
    pub penetration: f64,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub ended_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct LeaderboardRow {
    /// Posición, desde 1. Empates comparten posición.
    pub rank: u32,
    pub display_name: String,
    /// Casual semanal: el mejor pico de la semana.
    pub value: i64,
    pub is_me: bool,
}

/// `GET /casual/leaderboard/weekly`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct WeeklyLeaderboard {
    /// Lunes 00:00 UTC de la semana (RFC 3339).
    pub week_start: String,
    /// Los 100 primeros.
    pub top: Vec<LeaderboardRow>,
    /// La fila del usuario, si tiene partidas esta semana (aunque esté en `top`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub me: Option<LeaderboardRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct CasualRecords {
    pub runs: u32,
    /// Mejor pico de saldo en una partida.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub best_peak: Option<i64>,
    /// Más manos jugadas en una partida.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub most_hands: Option<u32>,
}

/// `GET /me/records`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct MyRecords {
    pub casual: CasualRecords,
}
