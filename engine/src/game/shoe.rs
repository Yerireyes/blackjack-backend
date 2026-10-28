//! Zapato con el orden real de las cartas. Solo vive en el servidor: el cliente
//! recibe como mucho la composición restante, nunca el orden.

use serde::{Deserialize, Serialize};

use super::card::{Card, Face, Suit};
use super::GameError;
use crate::rank::NUM_RANKS;
use crate::MIN_SHOE_VALUE;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameShoe {
    cards: Vec<Card>,
    /// Índice de la próxima carta a repartir.
    next: usize,
    decks: u8,
}

impl GameShoe {
    /// `decks` mazos completos, sin barajar.
    pub fn ordered(decks: u8) -> GameShoe {
        let mut cards = Vec::with_capacity(decks as usize * 52);
        for _ in 0..decks {
            for suit in Suit::ALL {
                for face in Face::ALL {
                    cards.push(Card { face, suit });
                }
            }
        }
        GameShoe {
            cards,
            next: 0,
            decks,
        }
    }

    /// Mazos nuevos barajados. `below(n)` debe devolver un entero uniforme en `0..n`
    /// sacado de un CSPRNG; el motor no depende de ninguna fuente de azar.
    pub fn shuffled(decks: u8, below: impl FnMut(usize) -> usize) -> GameShoe {
        let mut shoe = GameShoe::ordered(decks);
        shoe.shuffle(below);
        shoe
    }

    /// Junta todas las cartas y las baraja (Fisher–Yates).
    pub fn shuffle(&mut self, mut below: impl FnMut(usize) -> usize) {
        for i in (1..self.cards.len()).rev() {
            let j = below(i + 1);
            self.cards.swap(i, j);
        }
        self.next = 0;
    }

    /// Para tests: un zapato cuyas primeras cartas son `top`, en ese orden, y luego
    /// el resto de los mazos.
    pub fn stacked(decks: u8, top: &[Face]) -> GameShoe {
        let mut rest = GameShoe::ordered(decks).cards;
        let mut cards = Vec::with_capacity(rest.len());
        for &face in top {
            let i = rest
                .iter()
                .position(|c| c.face == face)
                .expect("quedan cartas de esa figura");
            cards.push(rest.remove(i));
        }
        cards.extend(rest);
        GameShoe {
            cards,
            next: 0,
            decks,
        }
    }

    pub fn decks(&self) -> u8 {
        self.decks
    }

    pub fn draw(&mut self) -> Result<Card, GameError> {
        let card = *self.cards.get(self.next).ok_or(GameError::ShoeEmpty)?;
        self.next += 1;
        Ok(card)
    }

    /// Fracción ya repartida, para el indicador del zapato y la penetración.
    pub fn dealt_fraction(&self) -> f64 {
        if self.cards.is_empty() {
            return 1.0;
        }
        self.next as f64 / self.cards.len() as f64
    }

    /// Cartas sin repartir por rango del motor.
    pub fn remaining_counts(&self) -> [u16; NUM_RANKS] {
        let mut counts = [0u16; NUM_RANKS];
        for c in &self.cards[self.next..] {
            counts[c.rank().index()] += 1;
        }
        counts
    }

    fn remaining_value(&self) -> u32 {
        self.cards[self.next..]
            .iter()
            .map(|c| c.rank().value() as u32)
            .sum()
    }

    /// Hay que rebarajar antes de la próxima ronda: se alcanzó la penetración o no
    /// alcanzan las cartas para una ronda completa (hasta `max_hands` manos de 31
    /// puntos y el dealer con 26) dejando las que el motor necesita para calcular.
    pub fn needs_shuffle(&self, penetration: f64, max_hands: u8) -> bool {
        let need = MIN_SHOE_VALUE + 31 * max_hands as u32 + 26;
        self.dealt_fraction() >= penetration || self.remaining_value() < need
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Generador determinista para los tests (no es para jugar).
    fn lcg(seed: u64) -> impl FnMut(usize) -> usize {
        let mut s = seed;
        move |n| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as usize) % n
        }
    }

    #[test]
    fn shoe_has_every_card_once_per_deck() {
        let shoe = GameShoe::shuffled(6, lcg(1));
        assert_eq!(shoe.cards.len(), 312);
        let counts = shoe.remaining_counts();
        assert_eq!(counts[0], 24);
        assert_eq!(counts[9], 96);
        let mut seen = HashSet::new();
        for c in &shoe.cards {
            assert!(seen.insert(*c) || shoe.decks > 1);
        }
        assert_eq!(seen.len(), 52);
    }

    #[test]
    fn never_deals_a_card_twice() {
        let mut shoe = GameShoe::shuffled(1, lcg(7));
        let mut seen = HashSet::new();
        while let Ok(c) = shoe.draw() {
            assert!(seen.insert(c), "{c:?} repetida");
        }
        assert_eq!(seen.len(), 52);
        assert_eq!(shoe.draw(), Err(GameError::ShoeEmpty));
    }

    #[test]
    fn stacked_puts_the_cards_on_top() {
        let mut shoe = GameShoe::stacked(1, &[Face::K, Face::Ace]);
        assert_eq!(shoe.draw().unwrap().face, Face::K);
        assert_eq!(shoe.draw().unwrap().face, Face::Ace);
        assert_eq!(shoe.remaining_counts().iter().sum::<u16>(), 50);
    }

    #[test]
    fn reshuffle_at_penetration_or_when_short() {
        let mut shoe = GameShoe::ordered(6);
        assert!(!shoe.needs_shuffle(0.75, 4));
        shoe.next = 234; // 75 % de 312
        assert!(shoe.needs_shuffle(0.75, 4));
        let mut one = GameShoe::ordered(1);
        one.next = 30;
        assert!(
            one.needs_shuffle(1.0, 4),
            "quedan pocas cartas para una ronda"
        );
        one.shuffle(lcg(3));
        assert_eq!(one.next, 0);
    }
}
