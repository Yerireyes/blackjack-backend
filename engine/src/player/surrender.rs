use crate::outcome::Outcome;

/// Rendición tardía: se pierde media apuesta, siempre.
pub fn surrender() -> Outcome {
    Outcome {
        ev: -0.5,
        win: 0.0,
        push: 0.0,
        lose: 1.0,
    }
}
