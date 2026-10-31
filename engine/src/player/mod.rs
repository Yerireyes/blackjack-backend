//! Acciones del jugador. Todas comparten un [`Solver`] con la memoización del
//! dealer y de la recursión pedir/plantarse para una carta visible y unas reglas.

mod double;
mod hit;
mod split;
mod stand;
mod surrender;

pub use stand::stand_vs;
pub use surrender::surrender;

use rustc_hash::FxHashMap;

use crate::dealer::{DealerDist, DealerSolver};
use crate::hand::Hand;
use crate::hole::forbidden_rank;
use crate::outcome::Outcome;
use crate::rank::Rank;
use crate::rules::Rules;
use crate::shoe::Shoe;

/// Decisión del jugador en un nodo (la usa también el simulador).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Stand,
    Hit,
    Double,
}

pub struct Solver {
    rules: Rules,
    dealer: DealerSolver,
    forbidden: Option<Rank>,
    /// V(t, S) y si la rama elegida es pedir.
    memo_best: FxHashMap<u128, (Outcome, bool)>,
}

/// Clave de memo: zapato en los 80 bits bajos, suma dura y As encima.
fn node_key(shoe: &Shoe, hand: Hand) -> u128 {
    shoe.key() | (hand.hard as u128) << 80 | (hand.has_ace as u128) << 88
}

impl Solver {
    /// `rules` debe estar validado.
    pub fn new(rules: &Rules, up: Rank) -> Solver {
        let forbidden = forbidden_rank(up, rules);
        Solver {
            rules: rules.clone(),
            dealer: DealerSolver::new(up, rules.dealer_hits_soft17, forbidden),
            forbidden,
            memo_best: FxHashMap::default(),
        }
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn up(&self) -> Rank {
        self.dealer.up()
    }

    pub fn forbidden(&self) -> Option<Rank> {
        self.forbidden
    }

    /// Distribución final del dealer vista desde este zapato (oculta incluida).
    pub fn dealer_dist(&mut self, shoe: &Shoe) -> DealerDist {
        self.dealer.dist(shoe)
    }

    pub fn dealer_mut(&mut self) -> &mut DealerSolver {
        &mut self.dealer
    }

    /// Nodos memoizados (jugador y dealer): mide cuánta memoria ocupa.
    pub fn memo_len(&self) -> usize {
        self.memo_best.len() + self.dealer.memo_len()
    }
}
