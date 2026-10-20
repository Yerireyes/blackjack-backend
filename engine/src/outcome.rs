/// Resultado de una acción: EV por unidad apostada y probabilidad de cada desenlace.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    pub ev: f64,
    pub win: f64,
    pub push: f64,
    pub lose: f64,
}

impl Outcome {
    pub const BUST: Outcome = Outcome {
        ev: -1.0,
        win: 0.0,
        push: 0.0,
        lose: 1.0,
    };

    /// `self += p * other`
    pub fn add_scaled(&mut self, other: &Outcome, p: f64) {
        self.ev += p * other.ev;
        self.win += p * other.win;
        self.push += p * other.push;
        self.lose += p * other.lose;
    }

    /// Mismas probabilidades con la apuesta multiplicada por `k`.
    pub fn with_stake(mut self, k: f64) -> Outcome {
        self.ev *= k;
        self
    }

    pub fn prob_sum(&self) -> f64 {
        self.win + self.push + self.lose
    }
}
