//! Simulación Monte Carlo de un estado concreto (zapato, mano, carta visible).
//!
//! Reparte la carta oculta y las siguientes al azar desde ese mismo zapato,
//! respetando el peek, y juega con la estrategia que eligió el motor en cada nodo.

use engine::hole::forbidden_rank;
use engine::{Action, Decision, Hand, Rank, Rules, Shoe, Solver};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub rules: Rules,
    /// Cartas sin ver (incluye la oculta).
    pub shoe: Shoe,
    pub hand: Vec<Rank>,
    pub up: Rank,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub n: u64,
    pub sum: f64,
    pub sum_sq: f64,
}

impl Stats {
    fn push(&mut self, x: f64) {
        self.n += 1;
        self.sum += x;
        self.sum_sq += x * x;
    }

    fn merge(&mut self, o: &Stats) {
        self.n += o.n;
        self.sum += o.sum;
        self.sum_sq += o.sum_sq;
    }

    pub fn mean(&self) -> f64 {
        self.sum / self.n as f64
    }

    /// Error estándar de la media.
    pub fn std_err(&self) -> f64 {
        let n = self.n as f64;
        let var = (self.sum_sq / n - self.mean().powi(2)) * n / (n - 1.0);
        (var / n).sqrt()
    }
}

/// Saca una carta uniforme del zapato.
fn draw(shoe: &mut Shoe, rng: &mut ChaCha8Rng) -> Rank {
    let mut target = rng.random_range(0..shoe.total());
    for r in Rank::all() {
        let c = shoe.count(r);
        if target < c {
            shoe.remove(r);
            return r;
        }
        target -= c;
    }
    unreachable!("zapato vacío")
}

/// Una mano simulada. `real` es el zapato físico (sin la oculta). Las decisiones
/// se toman con un zapato "vista" (con la oculta dentro) que lleva quien llama.
struct Table<'a> {
    solver: &'a mut Solver,
    rng: &'a mut ChaCha8Rng,
    real: Shoe,
}

impl Table<'_> {
    /// Saca una carta del zapato real y la descuenta de la vista.
    fn deal(&mut self, view: &mut Shoe) -> Rank {
        let c = draw(&mut self.real, self.rng);
        view.remove(c);
        c
    }

    /// Juega pedir/plantarse con la estrategia del motor según `view`.
    fn play_out(&mut self, mut hand: Hand, view: &mut Shoe) -> Hand {
        while !hand.is_bust() && hand.total() < 21 && self.solver.should_hit(view, hand) {
            hand = hand.with_card(self.deal(view));
        }
        hand
    }

    /// Juega las manos que salen de dividir `card`, con re-división hasta
    /// `max_hands`. Cada mano decide con la estrategia post-división: el zapato
    /// justo después de dividir (`base`) menos sus propias cartas.
    fn play_split(&mut self, card: Rank, max_hands: u8, base: Shoe, out: &mut Vec<(Hand, f64)>) {
        let mut pending = 2u8;
        let mut hands = 2u8;
        while pending > 0 {
            let mut view = base;
            let c = self.deal(&mut view);
            if c == card && hands < max_hands {
                // La mano vuelve a ser un par: se re-divide y queda otra mano pendiente.
                hands += 1;
                pending += 1;
                continue;
            }
            pending -= 1;
            let h = Hand::EMPTY.with_card(card).with_card(c);
            let (_, decision) = self.solver.split_hand_value(&view, h, card.is_ace());
            out.push(match decision {
                Decision::Stand => (h, 1.0),
                Decision::Hit => {
                    let c2 = self.deal(&mut view);
                    (self.play_out(h.with_card(c2), &mut view), 1.0)
                }
                Decision::Double => (h.with_card(self.deal(&mut view)), 2.0),
            });
        }
    }
}

/// Juega la mano del dealer desde (visible, oculta) con el zapato físico.
fn dealer_final(
    up: Rank,
    hole: Rank,
    rules: &Rules,
    shoe: &mut Shoe,
    rng: &mut ChaCha8Rng,
) -> Option<Hand> {
    let mut hand = Hand::from_cards(&[up, hole]);
    if hand.is_two_card_21() {
        return None; // blackjack
    }
    loop {
        let t = hand.total();
        let stands = t >= 18 || (t == 17 && !(hand.is_soft() && rules.dealer_hits_soft17));
        if hand.is_bust() || stands {
            return Some(hand);
        }
        hand = hand.with_card(draw(shoe, rng));
    }
}

/// Resultado de una mano del jugador (no natural) con apuesta `stake`.
fn settle(player: Hand, stake: f64, dealer: Option<Hand>) -> f64 {
    if player.is_bust() {
        return -stake;
    }
    match dealer {
        None => -stake,
        Some(d) if d.is_bust() => stake,
        Some(d) => match player.total().cmp(&d.total()) {
            std::cmp::Ordering::Greater => stake,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Less => -stake,
        },
    }
}

/// Simula `n` manos de `case` tomando `action` en la raíz, en un solo hilo.
pub fn simulate_serial(case: &Case, action: Action, n: u64, seed: u64) -> Stats {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut solver = Solver::new(&case.rules, case.up);
    let forbidden = forbidden_rank(case.up, &case.rules);
    let start = Hand::from_cards(&case.hand);
    let natural = start.is_two_card_21();
    let mut stats = Stats::default();

    for _ in 0..n {
        let mut real = case.shoe;
        // Oculta condicionada al peek: si sale el rango prohibido se devuelve y se repite.
        let hole = loop {
            let h = draw(&mut real, &mut rng);
            if Some(h) == forbidden {
                real.add(h);
                continue;
            }
            break h;
        };
        let mut t = Table {
            solver: &mut solver,
            rng: &mut rng,
            real,
        };
        let mut view = case.shoe;

        // Manos jugadas: (mano final, apuesta).
        let mut hands: Vec<(Hand, f64)> = Vec::with_capacity(2);
        match action {
            Action::Surrender => {
                stats.push(-0.5);
                continue;
            }
            Action::Stand if natural => {
                let mut real = t.real;
                let dealer = dealer_final(case.up, hole, &case.rules, &mut real, &mut rng);
                stats.push(if dealer.is_none() {
                    0.0
                } else {
                    case.rules.blackjack_pays
                });
                continue;
            }
            Action::Stand => hands.push((start, 1.0)),
            Action::Hit => {
                let h = start.with_card(t.deal(&mut view));
                hands.push((t.play_out(h, &mut view), 1.0));
            }
            Action::Double => {
                let h = start.with_card(t.deal(&mut view));
                hands.push((h, 2.0));
            }
            Action::Split => {
                let card = case.hand[0];
                let max_hands = if card.is_ace() && !case.rules.resplit_aces {
                    2
                } else {
                    case.rules.max_split_hands
                };
                t.play_split(card, max_hands, case.shoe, &mut hands);
            }
        }
        let mut real = t.real;
        let dealer = dealer_final(case.up, hole, &case.rules, &mut real, &mut rng);
        stats.push(hands.iter().map(|&(h, s)| settle(h, s, dealer)).sum());
    }
    stats
}

/// Igual que [`simulate_serial`] repartido en `threads` hilos (semillas seed+i).
pub fn simulate(case: &Case, action: Action, n: u64, seed: u64, threads: usize) -> Stats {
    let threads = threads.max(1) as u64;
    let mut total = Stats::default();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let per = n / threads + u64::from(i < n % threads);
                s.spawn(move || simulate_serial(case, action, per, seed.wrapping_add(i)))
            })
            .collect();
        for h in handles {
            total.merge(&h.join().expect("hilo de simulación"));
        }
    });
    total
}
