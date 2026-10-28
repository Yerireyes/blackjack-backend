//! Cartas físicas (con figura y palo) para repartir en el servidor. El motor de
//! probabilidades solo ve el [`Rank`]: J, Q y K son un 10.

use serde::{Deserialize, Serialize};

use crate::hand::Hand;
use crate::rank::Rank;

/// Figura de la carta, como la escribe el front: "A", "2" … "10", "J", "Q", "K".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum Face {
    #[serde(rename = "A")]
    Ace,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "4")]
    Four,
    #[serde(rename = "5")]
    Five,
    #[serde(rename = "6")]
    Six,
    #[serde(rename = "7")]
    Seven,
    #[serde(rename = "8")]
    Eight,
    #[serde(rename = "9")]
    Nine,
    #[serde(rename = "10")]
    Ten,
    J,
    Q,
    K,
}

impl Face {
    pub const ALL: [Face; 13] = [
        Face::Ace,
        Face::Two,
        Face::Three,
        Face::Four,
        Face::Five,
        Face::Six,
        Face::Seven,
        Face::Eight,
        Face::Nine,
        Face::Ten,
        Face::J,
        Face::Q,
        Face::K,
    ];

    pub fn rank(self) -> Rank {
        let i = Face::ALL
            .iter()
            .position(|&f| f == self)
            .expect("figura conocida");
        Rank::new(i.min(9) as u8).expect("índice de rango válido")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub enum Suit {
    S,
    H,
    D,
    C,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::S, Suit::H, Suit::D, Suit::C];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub face: Face,
    pub suit: Suit,
}

impl Card {
    pub fn rank(self) -> Rank {
        self.face.rank()
    }
}

/// Valor de una mano como lo muestra la mesa (igual que `handValue` del front).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandValue {
    /// Mejor total: un As cuenta 11 si no se pasa.
    pub total: u8,
    pub soft: bool,
    pub bust: bool,
    /// Dos cartas que suman 21, sin venir de dividir.
    pub blackjack: bool,
}

pub fn hand_value(cards: &[Card], from_split: bool) -> HandValue {
    let ranks: Vec<Rank> = cards.iter().map(|c| c.rank()).collect();
    let h = Hand::from_cards(&ranks);
    HandValue {
        total: h.total(),
        soft: h.is_soft(),
        bust: h.is_bust(),
        blackjack: !from_split && h.is_two_card_21(),
    }
}

/// Dos cartas del mismo rango del motor (un 10 y una K son par).
pub fn is_pair(cards: &[Card]) -> Option<Rank> {
    match cards {
        [a, b] if a.rank() == b.rank() => Some(a.rank()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(faces: &[Face]) -> Vec<Card> {
        faces
            .iter()
            .map(|&face| Card {
                face,
                suit: Suit::S,
            })
            .collect()
    }

    #[test]
    fn faces_map_to_engine_ranks() {
        assert_eq!(Face::Ace.rank(), Rank::ACE);
        assert_eq!(Face::Nine.rank().index(), 8);
        for f in [Face::Ten, Face::J, Face::Q, Face::K] {
            assert_eq!(f.rank(), Rank::TEN);
        }
    }

    #[test]
    fn hand_values() {
        let v = hand_value(&cards(&[Face::Ace, Face::K]), false);
        assert!(v.blackjack && v.total == 21);
        assert!(!hand_value(&cards(&[Face::Ace, Face::K]), true).blackjack);
        let v = hand_value(&cards(&[Face::Ace, Face::Six]), false);
        assert_eq!((v.total, v.soft), (17, true));
        let v = hand_value(&cards(&[Face::K, Face::Six, Face::Nine]), false);
        assert!(v.bust);
        assert_eq!(is_pair(&cards(&[Face::Ten, Face::Q])), Some(Rank::TEN));
        assert_eq!(is_pair(&cards(&[Face::Nine, Face::Q])), None);
    }

    #[test]
    fn serializes_like_the_front() {
        let c = Card {
            face: Face::Ten,
            suit: Suit::H,
        };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"face":"10","suit":"H"}"#
        );
    }
}
