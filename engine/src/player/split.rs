//! Dividir.
//!
//! Estrategia después de dividir ("post-división"): cada mano decide de forma
//! óptima para sí misma con el zapato que había justo después de dividir menos sus
//! propias cartas. No usa las cartas que recibió la otra mano para decidir (el
//! jugador real, que sí las ve, solo puede hacerlo un poco mejor).
//!
//! - **Dos manos sin re-división** ([`Solver::split_two_exact`]): EV exacto de esa
//!   estrategia. Las cartas de cada mano se sacan del zapato real (con las de la
//!   mano anterior ya fuera): se recorren todos los estados finales de la mano 1,
//!   desde cada uno todos los de la mano 2, y el dealer juega con el zapato que
//!   queda. Se agrupa por zapato final para calcular la distribución del dealer una
//!   sola vez por zapato.
//! - **Re-división** ([`Solver::split`] con más de dos manos posibles): aproximada.
//!   Se calcula el número esperado de manos con la probabilidad de volver a sacar el
//!   mismo rango (siempre se re-divide cuando se puede) y se suma la ganancia sobre
//!   el caso de dos manos al resultado exacto de dos manos.

use rustc_hash::FxHashMap;

use super::{Decision, Solver};
use crate::dealer::{BLACKJACK, BUST, D17, D21};
use crate::hand::Hand;
use crate::hole::next_card_probs;
use crate::outcome::Outcome;
use crate::rank::Rank;
use crate::shoe::Shoe;

/// Código de total final de una mano dividida: 4..=21, o pasada.
const BUST_CODE: u8 = 22;
const N_DEALER: usize = 7;

/// Presupuesto de trabajo del cálculo exacto de dos manos, en unidades de ~1 µs
/// medidas en una laptop de desarrollo. Es un conteo (no un reloj) para que el
/// resultado sea determinista. Si se supera, se usa la aproximación.
pub const SPLIT_EXACT_BUDGET: u64 = 300_000;

/// Costo relativo de una distribución del dealer en un zapato nuevo, por carta
/// visible (A, 2, ..., 10): con cartas bajas el árbol del dealer es más profundo.
const DEALER_COST: [u64; 10] = [30, 60, 52, 45, 38, 30, 25, 20, 18, 15];

/// Estado final de una mano dividida.
#[derive(Clone, Copy, Debug)]
struct Terminal {
    shoe: Shoe,
    code: u8,
    stake: u8,
    p: f64,
}

/// (total final, apuesta, probabilidad) de la mano 1, agrupados por zapato.
type HandEnd = (u8, u8, f64);

/// Pagos de una mano con total `code` frente a cada resultado del dealer:
/// (ganancia por unidad, gana, empata, pierde).
fn payoff_table(code: u8) -> [[f64; N_DEALER]; 4] {
    let mut t = [[0.0; N_DEALER]; 4];
    #[allow(clippy::needless_range_loop)] // k es el índice del resultado del dealer
    for k in 0..N_DEALER {
        let (win, push) = if code == BUST_CODE || k == BLACKJACK {
            (false, false)
        } else if k == BUST {
            (true, false)
        } else {
            let dealer_total = 17 + (k - D17) as u8;
            debug_assert!(k <= D21);
            (code > dealer_total, code == dealer_total)
        };
        let lose = !win && !push;
        t[0][k] = if win {
            1.0
        } else if push {
            0.0
        } else {
            -1.0
        };
        t[1][k] = win as u8 as f64;
        t[2][k] = push as u8 as f64;
        t[3][k] = lose as u8 as f64;
    }
    t
}

/// Número esperado de manos que terminan (con la segunda carta distinta del par
/// mientras aún se puede re-dividir, y con cualquier carta cuando ya no), partiendo
/// de `pending` manos con una sola carta y `hands` manos en total, con tope `max`.
/// `p` es la probabilidad de que una mano reciba otra carta del mismo rango.
fn expected_hands(p: f64, pending: u8, hands: u8, max: u8) -> (f64, f64) {
    if pending == 0 {
        return (0.0, 0.0);
    }
    if hands >= max {
        return (0.0, pending as f64);
    }
    let (a1, e1) = expected_hands(p, pending + 1, hands + 1, max);
    let (a0, e0) = expected_hands(p, pending - 1, hands, max);
    (p * a1 + (1.0 - p) * (1.0 + a0), p * e1 + (1.0 - p) * e0)
}

impl Solver {
    /// Dividir el par `card`. `max_hands` es cuántas manos pueden salir de este
    /// par como mucho (2 si no se puede re-dividir). Devuelve el EV total (ambas
    /// manos) con `win`/`push`/`lose` promediados por mano.
    /// El booleano indica si la base de dos manos se calculó de forma exacta
    /// (si no cabía en el presupuesto se usa 2 * E1, que ignora la dependencia
    /// entre las cartas de las dos manos).
    pub fn split(&mut self, shoe: &Shoe, card: Rank, max_hands: u8) -> (Outcome, bool) {
        let (all, non_pair, p_pair) = self.split_hand_parts(shoe, card);
        let exact_two = self.split_two_exact(shoe, card);
        let base = exact_two.unwrap_or_else(|| all.with_stake(2.0));
        if max_hands <= 2 {
            return (base, exact_two.is_some());
        }
        // Ganancia aproximada de poder re-dividir, sobre el caso de dos manos.
        let (n_avail, n_exh) = expected_hands(p_pair, 2, 2, max_hands);
        let n = n_avail + n_exh;
        let mut with_resplit = Outcome::default();
        with_resplit.add_scaled(&non_pair, n_avail);
        with_resplit.add_scaled(&all, n_exh);
        let mut out = base;
        out.ev += with_resplit.ev - 2.0 * all.ev;
        // Probabilidades por mano: corrección que suma 0, así siguen sumando 1.
        out.win += with_resplit.win / n - all.win;
        out.push += with_resplit.push / n - all.push;
        out.lose += with_resplit.lose / n - all.lose;
        (out, false)
    }

    /// E1 de una mano dividida (todas las segundas cartas), la misma condicionada a
    /// que la segunda carta no sea del rango del par, y la probabilidad de que lo sea.
    fn split_hand_parts(&mut self, shoe: &Shoe, card: Rank) -> (Outcome, Outcome, f64) {
        let p = next_card_probs(shoe, self.forbidden);
        let start = Hand::EMPTY.with_card(card);
        let mut all = Outcome::default();
        let mut non_pair = Outcome::default();
        for r in Rank::all() {
            let w = p[r.index()];
            if w == 0.0 {
                continue;
            }
            let (v, _) = self.split_hand_value(&shoe.without(r), start.with_card(r), card.is_ace());
            all.add_scaled(&v, w);
            if r != card {
                non_pair.add_scaled(&v, w);
            }
        }
        let p_pair = p[card.index()];
        if p_pair < 1.0 {
            let k = 1.0 / (1.0 - p_pair);
            non_pair = Outcome {
                ev: non_pair.ev * k,
                win: non_pair.win * k,
                push: non_pair.push * k,
                lose: non_pair.lose * k,
            };
        }
        (all, non_pair, p_pair)
    }

    /// Mejor jugada para una mano dividida que ya recibió su segunda carta.
    /// Un 21 de dos cartas aquí no es blackjack.
    pub fn split_hand_value(
        &mut self,
        shoe: &Shoe,
        hand: Hand,
        split_aces: bool,
    ) -> (Outcome, Decision) {
        if split_aces && !self.rules.hit_split_aces {
            return (self.stand(shoe, hand), Decision::Stand);
        }
        let (best, hit) = self.best(shoe, hand);
        let mut choice = (best, if hit { Decision::Hit } else { Decision::Stand });
        if self.rules.double_after_split
            && self.rules.can_double_total(hand.total(), hand.is_soft())
        {
            let dbl = self.double(shoe, hand);
            if dbl.ev > choice.0.ev {
                choice = (dbl, Decision::Double);
            }
        }
        choice
    }

    /// EV exacto de dividir en exactamente dos manos (sin re-división), o `None`
    /// si el trabajo estimado supera [`SPLIT_EXACT_BUDGET`].
    pub fn split_two_exact(&mut self, shoe: &Shoe, card: Rank) -> Option<Outcome> {
        let tables: Vec<[[f64; N_DEALER]; 4]> = (0..=BUST_CODE).map(payoff_table).collect();

        // Mano 1, agrupada por zapato resultante.
        let mut by_shoe: FxHashMap<u128, (Shoe, Vec<HandEnd>)> = FxHashMap::default();
        let hand1_terminals = self.split_terminals(shoe, shoe, card);
        let n_terminals = hand1_terminals.len() as u64;
        for t in hand1_terminals {
            by_shoe
                .entry(t.shoe.key())
                .or_insert_with(|| (t.shoe, Vec::new()))
                .1
                .push((t.code, t.stake, t.p));
        }

        // Para cada zapato final: pesos lineales sobre la distribución del dealer.
        // [ev, gana, empata, pierde] x 7 resultados del dealer.
        let mut finals: FxHashMap<u128, (Shoe, [[f64; N_DEALER]; 4])> = FxHashMap::default();
        let mut groups: Vec<_> = by_shoe.into_values().collect();
        groups.sort_by_key(|(s, _)| s.key()); // orden fijo: resultado determinista

        // Presupuesto: enumerar la mano 2 cuesta ~ grupos x estados finales, y cada
        // zapato final una distribución del dealer (más cara con cartas visibles bajas).
        let enum_cost = groups.len() as u64 * n_terminals;
        let final_cost = DEALER_COST[self.up().index()];
        if enum_cost > SPLIT_EXACT_BUDGET {
            return None;
        }
        for (s1, hand1) in groups {
            if enum_cost + finals.len() as u64 * final_cost > SPLIT_EXACT_BUDGET {
                return None;
            }
            for t2 in self.split_terminals(&s1, shoe, card) {
                let entry = finals
                    .entry(t2.shoe.key())
                    .or_insert_with(|| (t2.shoe, [[0.0; N_DEALER]; 4]));
                let u2 = &tables[t2.code as usize];
                for &(code1, stake1, p1) in &hand1 {
                    let w = p1 * t2.p;
                    let u1 = &tables[code1 as usize];
                    for k in 0..N_DEALER {
                        entry.1[0][k] +=
                            w * (stake1 as f64 * u1[0][k] + t2.stake as f64 * u2[0][k]);
                        for m in 1..4 {
                            entry.1[m][k] += w * 0.5 * (u1[m][k] + u2[m][k]);
                        }
                    }
                }
            }
        }

        let mut finals: Vec<_> = finals.into_values().collect();
        finals.sort_by_key(|(s, _)| s.key());
        let mut acc = [0.0; 4];
        for (s2, weights) in finals {
            let d = self.dealer.dist(&s2);
            for (m, row) in weights.iter().enumerate() {
                acc[m] += row.iter().zip(d.p.iter()).map(|(w, p)| w * p).sum::<f64>();
            }
        }
        Some(Outcome {
            ev: acc[0],
            win: acc[1],
            push: acc[2],
            lose: acc[3],
        })
    }

    /// Todos los estados finales de una mano dividida que empieza con `card`,
    /// agregados por (zapato, total, apuesta). Las cartas salen de `draw` (zapato
    /// real) y las decisiones se toman con `decide` (zapato post-división) menos las
    /// cartas propias de la mano.
    fn split_terminals(&mut self, draw: &Shoe, decide: &Shoe, card: Rank) -> Vec<Terminal> {
        let mut acc: FxHashMap<u128, Terminal> = FxHashMap::default();
        let p = next_card_probs(draw, self.forbidden);
        let start = Hand::EMPTY.with_card(card);
        for r in Rank::all() {
            let w = p[r.index()];
            if w == 0.0 {
                continue;
            }
            let (d1, v1) = (draw.without(r), decide.without(r));
            let hand = start.with_card(r);
            match self.split_hand_value(&v1, hand, card.is_ace()).1 {
                Decision::Stand => push_terminal(&mut acc, d1, hand, 1, w),
                Decision::Double => {
                    let p2 = next_card_probs(&d1, self.forbidden);
                    for r2 in Rank::all() {
                        let w2 = p2[r2.index()];
                        if w2 > 0.0 {
                            push_terminal(&mut acc, d1.without(r2), hand.with_card(r2), 2, w * w2);
                        }
                    }
                }
                Decision::Hit => self.hit_terminals(&d1, &v1, hand, w, &mut acc),
            }
        }
        let mut out: Vec<Terminal> = acc.into_values().collect();
        out.sort_by_key(|t| (t.shoe.key(), t.code, t.stake));
        out
    }

    /// Pide una carta con `hand` y sigue la estrategia óptima hasta plantarse o pasarse.
    fn hit_terminals(
        &mut self,
        draw: &Shoe,
        decide: &Shoe,
        hand: Hand,
        w: f64,
        acc: &mut FxHashMap<u128, Terminal>,
    ) {
        let p = next_card_probs(draw, self.forbidden);
        for r in Rank::all() {
            let wr = p[r.index()];
            if wr == 0.0 {
                continue;
            }
            let (d, v) = (draw.without(r), decide.without(r));
            let next = hand.with_card(r);
            if !next.is_bust() && self.should_hit(&v, next) {
                self.hit_terminals(&d, &v, next, w * wr, acc);
            } else {
                push_terminal(acc, d, next, 1, w * wr);
            }
        }
    }
}

fn push_terminal(acc: &mut FxHashMap<u128, Terminal>, shoe: Shoe, hand: Hand, stake: u8, p: f64) {
    let code = if hand.is_bust() {
        BUST_CODE
    } else {
        hand.total()
    };
    let key = shoe.key() | (code as u128) << 80 | (stake as u128) << 88;
    acc.entry(key).and_modify(|t| t.p += p).or_insert(Terminal {
        shoe,
        code,
        stake,
        p,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_hands_counts() {
        // Sin re-división: siempre dos manos, ambas con cualquier carta.
        assert_eq!(expected_hands(0.3, 2, 2, 2), (0.0, 2.0));
        // Probabilidad 0 de volver a sacar el par: dos manos "disponibles".
        assert_eq!(expected_hands(0.0, 2, 2, 4), (2.0, 0.0));
        // Total esperado de manos crece con p y nunca pasa del tope.
        let (a, e) = expected_hands(0.1, 2, 2, 4);
        let (a2, e2) = expected_hands(0.3, 2, 2, 4);
        assert!(a + e > 2.0 && a2 + e2 > a + e && a2 + e2 <= 4.0);
    }
}
