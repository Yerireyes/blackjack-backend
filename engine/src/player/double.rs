//! Doblar(t, S) = 2 * sum_r P(r | S) * Plantarse(t + r, S - r)

use super::Solver;
use crate::hand::Hand;
use crate::hole::next_card_probs;
use crate::outcome::Outcome;
use crate::rank::Rank;
use crate::shoe::Shoe;

impl Solver {
    pub fn double(&mut self, shoe: &Shoe, hand: Hand) -> Outcome {
        let p = next_card_probs(shoe, self.forbidden);
        let mut out = Outcome::default();
        for r in Rank::all() {
            let w = p[r.index()];
            if w == 0.0 {
                continue;
            }
            let v = self.stand(&shoe.without(r), hand.with_card(r));
            out.add_scaled(&v, w);
        }
        out.with_stake(2.0)
    }
}
