//! Distribución de la carta oculta del dealer y de la siguiente carta del jugador.
//!
//! El zapato que recibe el motor incluye la carta oculta, porque el jugador no la
//! conoce. Si hay peek y la carta visible es As (o 10), y el dealer no tiene
//! blackjack, la oculta no puede ser 10 (o As): ese es el rango prohibido `f`.

use crate::rank::{Rank, NUM_RANKS};
use crate::rules::Rules;
use crate::shoe::Shoe;

/// Rango que la carta oculta no puede tener tras un peek sin blackjack.
pub fn forbidden_rank(up: Rank, rules: &Rules) -> Option<Rank> {
    if !rules.dealer_peeks {
        None
    } else if up == Rank::ACE {
        Some(Rank::TEN)
    } else if up == Rank::TEN {
        Some(Rank::ACE)
    } else {
        None
    }
}

/// q_h(S): probabilidad de que la oculta sea `h`.
///   q_h = n_h / (N - n_f) para h != f,  q_f = 0.
pub fn hole_probs(shoe: &Shoe, forbidden: Option<Rank>) -> [f64; NUM_RANKS] {
    let mut q = [0.0; NUM_RANKS];
    let n_f = forbidden.map_or(0, |f| shoe.count(f));
    let denom = (shoe.total() - n_f) as f64;
    if denom <= 0.0 {
        return q;
    }
    for r in Rank::all() {
        if Some(r) != forbidden {
            q[r.index()] = shoe.count(r) as f64 / denom;
        }
    }
    q
}

/// P(siguiente = r | S) = sum_h q_h(S) * (n_r - [h = r]) / (N - 1).
///
/// Forma cerrada de esa suma:
/// - sin rango prohibido se reduce a n_r / N;
/// - con rango prohibido f:
///   r != f:  n_r (N - n_f - 1) / ((N - n_f)(N - 1))
///   r == f:  n_f / (N - 1)
pub fn next_card_probs(shoe: &Shoe, forbidden: Option<Rank>) -> [f64; NUM_RANKS] {
    let mut p = [0.0; NUM_RANKS];
    let n = shoe.total() as f64;
    // La oculta ocupa una carta: el jugador necesita al menos otra.
    if shoe.total() < 2 {
        return p;
    }
    match forbidden {
        None => {
            for r in Rank::all() {
                p[r.index()] = shoe.count(r) as f64 / n;
            }
        }
        Some(f) => {
            let n_f = shoe.count(f) as f64;
            let free = n - n_f; // cartas que pueden ser la oculta
            if free <= 0.0 {
                return p; // estado imposible (el dealer tendría blackjack)
            }
            for r in Rank::all() {
                let n_r = shoe.count(r) as f64;
                p[r.index()] = if r == f {
                    n_f / (n - 1.0)
                } else {
                    n_r * (free - 1.0) / (free * (n - 1.0))
                };
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La forma cerrada debe coincidir con la suma explícita sobre la oculta.
    #[test]
    fn closed_form_matches_sum() {
        let shoes = [
            Shoe::full(1),
            Shoe::full(6).without(Rank::ACE),
            Shoe::new([3, 0, 1, 5, 2, 0, 4, 1, 2, 11]),
        ];
        for shoe in shoes {
            for f in [None, Some(Rank::TEN), Some(Rank::ACE)] {
                let q = hole_probs(&shoe, f);
                let closed = next_card_probs(&shoe, f);
                let n = shoe.total() as f64;
                for r in Rank::all() {
                    let explicit: f64 = Rank::all()
                        .map(|h| {
                            let nr = shoe.count(r) as f64 - if h == r { 1.0 } else { 0.0 };
                            q[h.index()] * nr.max(0.0) / (n - 1.0)
                        })
                        .sum();
                    assert!((explicit - closed[r.index()]).abs() < 1e-14, "{f:?} {r}");
                }
                let s: f64 = closed.iter().sum();
                assert!((s - 1.0).abs() < 1e-12);
            }
        }
    }
}
