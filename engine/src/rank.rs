use serde::{Deserialize, Serialize};

/// Rango de una carta: 0 = As, 1 = 2, ..., 8 = 9, 9 = cualquier carta de valor 10.
/// J, Q y K no existen para el motor.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
#[cfg_attr(feature = "ts", derive(tsify::Tsify))]
pub struct Rank(u8);

pub const NUM_RANKS: usize = 10;

impl Rank {
    pub const ACE: Rank = Rank(0);
    pub const TEN: Rank = Rank(9);

    pub fn new(index: u8) -> Option<Rank> {
        (index < NUM_RANKS as u8).then_some(Rank(index))
    }

    /// Todos los rangos en orden, del As al 10.
    pub fn all() -> impl Iterator<Item = Rank> {
        (0..NUM_RANKS as u8).map(Rank)
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// Valor duro de la carta (el As vale 1).
    pub fn value(self) -> u8 {
        if self.0 == 0 {
            1
        } else {
            self.0 + 1
        }
    }

    pub fn is_ace(self) -> bool {
        self.0 == 0
    }
}

impl TryFrom<u8> for Rank {
    type Error = String;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        Rank::new(v).ok_or_else(|| format!("rango de carta inválido: {v} (debe ser 0..9)"))
    }
}

impl From<Rank> for u8 {
    fn from(r: Rank) -> u8 {
        r.0
    }
}

impl std::fmt::Display for Rank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => write!(f, "A"),
            9 => write!(f, "10"),
            i => write!(f, "{}", i + 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(Rank::ACE.value(), 1);
        assert_eq!(Rank::new(1).unwrap().value(), 2);
        assert_eq!(Rank::new(8).unwrap().value(), 9);
        assert_eq!(Rank::TEN.value(), 10);
        assert!(Rank::new(10).is_none());
    }
}
