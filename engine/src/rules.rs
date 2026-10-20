use serde::{Deserialize, Serialize};

use crate::error::EngineError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum DoubleOn {
    Any,
    NineToEleven,
    TenToEleven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum Surrender {
    None,
    Late,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct Rules {
    pub decks: u8,
    pub dealer_hits_soft17: bool,
    pub double_after_split: bool,
    pub double_on: DoubleOn,
    pub max_split_hands: u8,
    pub resplit_aces: bool,
    pub hit_split_aces: bool,
    pub surrender: Surrender,
    pub dealer_peeks: bool,
    pub blackjack_pays: f64,
    /// La usa el front para rebarajar; el motor la ignora.
    pub penetration: f64,
}

pub const MAX_DECKS: u8 = 8;

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            decks: 6,
            dealer_hits_soft17: false,
            double_after_split: true,
            double_on: DoubleOn::Any,
            max_split_hands: 4,
            resplit_aces: false,
            hit_split_aces: false,
            surrender: Surrender::None,
            dealer_peeks: true,
            blackjack_pays: 1.5,
            penetration: 0.75,
        }
    }
}

impl Rules {
    pub fn validate(&self) -> Result<(), EngineError> {
        let bad = |m: &str| Err(EngineError::InvalidRules(m.to_string()));
        if self.decks == 0 || self.decks > MAX_DECKS {
            return bad(&format!("decks debe estar entre 1 y {MAX_DECKS}"));
        }
        if self.max_split_hands == 0 || self.max_split_hands > 8 {
            return bad("maxSplitHands debe estar entre 1 y 8");
        }
        if !self.blackjack_pays.is_finite()
            || self.blackjack_pays <= 0.0
            || self.blackjack_pays > 3.0
        {
            return bad("blackjackPays debe estar en (0, 3]");
        }
        if !self.penetration.is_finite() || !(0.0..=1.0).contains(&self.penetration) {
            return bad("penetration debe estar entre 0 y 1");
        }
        Ok(())
    }

    /// ¿Se puede doblar con este total de dos cartas?
    pub fn can_double_total(&self, total: u8, soft: bool) -> bool {
        match self.double_on {
            DoubleOn::Any => true,
            DoubleOn::NineToEleven => !soft && (9..=11).contains(&total),
            DoubleOn::TenToEleven => !soft && (10..=11).contains(&total),
        }
    }
}
