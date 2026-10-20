use super::Solver;
use crate::dealer::{DealerDist, BLACKJACK, BUST};
use crate::hand::Hand;
use crate::outcome::Outcome;
use crate::shoe::Shoe;

/// Plantarse con total `t` (≤ 21, no natural) frente a la distribución `d`.
pub fn stand_vs(d: &DealerDist, t: u8) -> Outcome {
    let mut win = d.p[BUST];
    let mut push = 0.0;
    let mut lose = d.p[BLACKJACK];
    for dealer_total in 17..=21u8 {
        let p = d.total(dealer_total);
        match dealer_total.cmp(&t) {
            std::cmp::Ordering::Less => win += p,
            std::cmp::Ordering::Equal => push += p,
            std::cmp::Ordering::Greater => lose += p,
        }
    }
    Outcome {
        ev: win - lose,
        win,
        push,
        lose,
    }
}

/// Blackjack natural del jugador: cobra `pays` salvo que el dealer también tenga (empate).
pub fn natural_vs(d: &DealerDist, pays: f64) -> Outcome {
    let push = d.p[BLACKJACK];
    let win = 1.0 - push;
    Outcome {
        ev: pays * win,
        win,
        push,
        lose: 0.0,
    }
}

impl Solver {
    /// Plantarse con `hand` (no natural) y zapato `shoe` (oculta incluida).
    pub fn stand(&mut self, shoe: &Shoe, hand: Hand) -> Outcome {
        if hand.is_bust() {
            return Outcome::BUST;
        }
        let d = self.dealer.dist(shoe);
        stand_vs(&d, hand.total())
    }

    pub fn natural(&mut self, shoe: &Shoe) -> Outcome {
        let d = self.dealer.dist(shoe);
        natural_vs(&d, self.rules.blackjack_pays)
    }
}
