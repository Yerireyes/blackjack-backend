//! V(t, S) = max{ Plantarse(t, S), sum_r P(r | S) V(t + r, S - r) }
//! Después de pedir solo existen pedir o plantarse. Empate exacto → plantarse.

use super::{node_key, Solver};
use crate::hand::Hand;
use crate::hole::next_card_probs;
use crate::outcome::Outcome;
use crate::rank::Rank;
use crate::shoe::Shoe;

impl Solver {
    /// Valor óptimo de la mano jugando solo pedir/plantarse, y si conviene pedir.
    pub fn best(&mut self, shoe: &Shoe, hand: Hand) -> (Outcome, bool) {
        if hand.is_bust() {
            return (Outcome::BUST, false);
        }
        let stand = self.stand(shoe, hand);
        // Con 21 pedir nunca mejora a plantarse.
        if hand.total() == 21 {
            return (stand, false);
        }
        let key = node_key(shoe, hand);
        if let Some(v) = self.memo_best.get(&key) {
            return *v;
        }
        let hit = self.hit(shoe, hand);
        let v = if hit.ev > stand.ev {
            (hit, true)
        } else {
            (stand, false)
        };
        self.memo_best.insert(key, v);
        v
    }

    /// Rama de pedir: una carta y luego juego óptimo.
    pub fn hit(&mut self, shoe: &Shoe, hand: Hand) -> Outcome {
        let p = next_card_probs(shoe, self.forbidden);
        let mut out = Outcome::default();
        for r in Rank::all() {
            let w = p[r.index()];
            if w == 0.0 {
                continue;
            }
            let next = hand.with_card(r);
            let v = if next.is_bust() {
                Outcome::BUST
            } else {
                self.best(&shoe.without(r), next).0
            };
            out.add_scaled(&v, w);
        }
        out
    }

    /// ¿La estrategia óptima pide en este nodo?
    pub fn should_hit(&mut self, shoe: &Shoe, hand: Hand) -> bool {
        self.best(shoe, hand).1
    }

    /// Probabilidad de pasarse con la próxima carta.
    pub fn p_bust_on_hit(&self, shoe: &Shoe, hand: Hand) -> f64 {
        let p = next_card_probs(shoe, self.forbidden);
        let bust: f64 = Rank::all()
            .filter(|&r| hand.with_card(r).is_bust())
            .map(|r| p[r.index()])
            .sum();
        // Con 21 duro se pasan todas las cartas y la suma puede dar 1 + ε por redondeo.
        bust.clamp(0.0, 1.0)
    }
}
