//! Distribución final del dealer.
//!
//! F(d, S) = sum_r (n_r / N) * F(d + r, S - r), memoizada por (total, suave, S.key()).
//! D(up, S) = sum_h q_h(S) * F(up + h, S - h), con q de la carta oculta (ver `hole`).

use rustc_hash::FxHashMap;

use crate::hand::Hand;
use crate::hole::hole_probs;
use crate::rank::Rank;
use crate::shoe::Shoe;

pub const D17: usize = 0;
pub const D21: usize = 4;
pub const BLACKJACK: usize = 5;
pub const BUST: usize = 6;

/// Probabilidades de: 17, 18, 19, 20, 21, Blackjack, Bust.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DealerDist {
    pub p: [f64; 7],
}

impl DealerDist {
    fn point(i: usize) -> DealerDist {
        let mut d = DealerDist::default();
        d.p[i] = 1.0;
        d
    }

    fn add_scaled(&mut self, other: &DealerDist, w: f64) {
        for (a, b) in self.p.iter_mut().zip(other.p.iter()) {
            *a += w * b;
        }
    }

    /// Probabilidad de que el dealer termine con `total` (17..=21, sin contar blackjack).
    pub fn total(&self, total: u8) -> f64 {
        match total {
            17..=21 => self.p[D17 + (total - 17) as usize],
            _ => 0.0,
        }
    }

    pub fn sum(&self) -> f64 {
        self.p.iter().sum()
    }
}

/// Calculador del dealer para una carta visible y unas reglas fijas.
/// Conserva la memoización entre llamadas con zapatos distintos.
pub struct DealerSolver {
    up: Rank,
    hits_soft17: bool,
    forbidden: Option<Rank>,
    memo_play: FxHashMap<u128, DealerDist>,
    memo_dist: FxHashMap<u128, DealerDist>,
}

impl DealerSolver {
    pub fn new(up: Rank, hits_soft17: bool, forbidden: Option<Rank>) -> DealerSolver {
        DealerSolver {
            up,
            hits_soft17,
            forbidden,
            memo_play: FxHashMap::default(),
            memo_dist: FxHashMap::default(),
        }
    }

    pub fn up(&self) -> Rank {
        self.up
    }

    pub fn forbidden(&self) -> Option<Rank> {
        self.forbidden
    }

    /// ¿El dealer se planta con esta mano?
    pub fn stands(&self, hand: Hand) -> bool {
        let t = hand.total();
        t >= 18 || (t == 17 && !(hand.is_soft() && self.hits_soft17))
    }

    /// D(up, S): distribución final con la oculta desconocida dentro de `shoe`.
    pub fn dist(&mut self, shoe: &Shoe) -> DealerDist {
        let key = shoe.key();
        if let Some(d) = self.memo_dist.get(&key) {
            return *d;
        }
        let q = hole_probs(shoe, self.forbidden);
        let mut out = DealerDist::default();
        for h in Rank::all() {
            let w = q[h.index()];
            if w == 0.0 {
                continue;
            }
            let d = self.with_hole(h, &shoe.without(h));
            out.add_scaled(&d, w);
        }
        self.memo_dist.insert(key, out);
        out
    }

    /// Distribución final conociendo la oculta (`shoe` ya no la contiene).
    pub fn with_hole(&mut self, hole: Rank, shoe: &Shoe) -> DealerDist {
        let hand = Hand::from_cards(&[self.up, hole]);
        if hand.is_two_card_21() {
            DealerDist::point(BLACKJACK)
        } else {
            self.play(hand, shoe)
        }
    }

    /// Distribución final conociendo todas las cartas del dealer (visible, oculta y
    /// las que ya sacó, en orden); `shoe` ya no las contiene.
    pub fn with_cards(&mut self, cards: &[Rank], shoe: &Shoe) -> DealerDist {
        let hand = Hand::from_cards(cards);
        if hand.is_two_card_21() {
            DealerDist::point(BLACKJACK)
        } else {
            self.play(hand, shoe)
        }
    }

    /// Probabilidad de pasarse con la próxima carta, o `None` si las reglas no lo
    /// obligan a pedir (ya se plantó, se pasó o tiene blackjack).
    pub fn p_bust_next(&self, cards: &[Rank], shoe: &Shoe) -> Option<f64> {
        let hand = Hand::from_cards(cards);
        if hand.is_bust() || hand.is_two_card_21() || self.stands(hand) || shoe.total() == 0 {
            return None;
        }
        let n = shoe.total() as f64;
        Some(
            Rank::all()
                .filter(|&r| hand.with_card(r).is_bust())
                .map(|r| shoe.count(r) as f64 / n)
                .sum(),
        )
    }

    /// F(d, S) desde una mano de dos o más cartas que ya no puede ser blackjack.
    fn play(&mut self, hand: Hand, shoe: &Shoe) -> DealerDist {
        if hand.is_bust() {
            return DealerDist::point(BUST);
        }
        if self.stands(hand) {
            return DealerDist::point(D17 + (hand.total() - 17) as usize);
        }
        let key = shoe.key() | (hand.hard as u128) << 80 | (hand.has_ace as u128) << 88;
        if let Some(d) = self.memo_play.get(&key) {
            return *d;
        }
        let mut out = DealerDist::default();
        let n = shoe.total() as f64;
        // Con la validación de tamaño mínimo del zapato esto no ocurre; si ocurre,
        // la distribución queda incompleta y los tests de suma 1 lo detectan.
        debug_assert!(n > 0.0, "el dealer necesita carta y el zapato está vacío");
        for r in Rank::all() {
            let c = shoe.count(r);
            if c == 0 {
                continue;
            }
            let d = self.play(hand.with_card(r), &shoe.without(r));
            out.add_scaled(&d, c as f64 / n);
        }
        self.memo_play.insert(key, out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hole::forbidden_rank;
    use crate::rules::Rules;

    #[test]
    fn sums_to_one_and_peek_removes_blackjack() {
        let rules = Rules {
            decks: 1,
            ..Rules::default()
        };
        for up in Rank::all() {
            let shoe = Shoe::full(1).without(up);
            let mut s = DealerSolver::new(up, false, forbidden_rank(up, &rules));
            let d = s.dist(&shoe);
            assert!((d.sum() - 1.0).abs() < 1e-12);
            assert_eq!(d.p[BLACKJACK], 0.0);
        }
        let mut s = DealerSolver::new(Rank::ACE, false, None);
        let d = s.dist(&Shoe::full(1).without(Rank::ACE));
        assert!((d.p[BLACKJACK] - 16.0 / 51.0).abs() < 1e-15);
    }
}
