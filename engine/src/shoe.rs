use crate::rank::{Rank, NUM_RANKS};

/// Cartas que quedan en el zapato, por rango (ver [`Rank`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shoe {
    counts: [u16; NUM_RANKS],
    total: u16,
}

impl Shoe {
    pub fn new(counts: [u16; NUM_RANKS]) -> Shoe {
        let total = counts.iter().sum();
        Shoe { counts, total }
    }

    /// Zapato completo de `decks` mazos.
    pub fn full(decks: u8) -> Shoe {
        let mut counts = [0u16; NUM_RANKS];
        for r in Rank::all() {
            counts[r.index()] = Shoe::capacity(decks, r);
        }
        Shoe::new(counts)
    }

    /// Cuántas cartas de rango `r` tiene un zapato completo de `decks` mazos.
    pub fn capacity(decks: u8, r: Rank) -> u16 {
        let per_deck = if r == Rank::TEN { 16 } else { 4 };
        per_deck * decks as u16
    }

    pub fn counts(&self) -> &[u16; NUM_RANKS] {
        &self.counts
    }

    pub fn count(&self, r: Rank) -> u16 {
        self.counts[r.index()]
    }

    pub fn total(&self) -> u16 {
        self.total
    }

    /// Suma de los valores duros de las cartas (As = 1).
    pub fn value_sum(&self) -> u32 {
        Rank::all()
            .map(|r| self.count(r) as u32 * r.value() as u32)
            .sum()
    }

    /// Probabilidad simple de sacar `r`: n_r / N.
    pub fn prob(&self, r: Rank) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.counts[r.index()] as f64 / self.total as f64
        }
    }

    pub fn remove(&mut self, r: Rank) {
        debug_assert!(
            self.counts[r.index()] > 0,
            "se sacó un {r} de un zapato sin {r}"
        );
        self.counts[r.index()] -= 1;
        self.total -= 1;
    }

    pub fn add(&mut self, r: Rank) {
        self.counts[r.index()] += 1;
        self.total += 1;
    }

    /// Copia del zapato sin una carta de rango `r`.
    pub fn without(&self, r: Rank) -> Shoe {
        let mut s = *self;
        s.remove(r);
        s
    }

    /// Los 10 conteos empaquetados (8 bits cada uno) en los 80 bits bajos.
    /// Con hasta 8 mazos el rango 10 tiene 128 cartas, así que cabe en 8 bits.
    pub fn key(&self) -> u128 {
        self.counts
            .iter()
            .enumerate()
            .fold(0u128, |k, (i, &c)| k | ((c as u128 & 0xFF) << (8 * i)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_shoe() {
        let s = Shoe::full(1);
        assert_eq!(s.total(), 52);
        assert_eq!(s.count(Rank::TEN), 16);
        assert_eq!(s.count(Rank::ACE), 4);
        assert_eq!(Shoe::full(8).total(), 416);
        assert_eq!(
            Shoe::full(1).value_sum(),
            4 * (1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9) + 16 * 10
        );
    }

    #[test]
    fn remove_add_roundtrip() {
        let mut s = Shoe::full(2);
        let original = s;
        s.remove(Rank::ACE);
        s.remove(Rank::TEN);
        assert_eq!(s.total(), 102);
        assert_eq!(s.count(Rank::ACE), 7);
        assert_ne!(s.key(), original.key());
        s.add(Rank::TEN);
        s.add(Rank::ACE);
        assert_eq!(s, original);
        assert_eq!(s.key(), original.key());
    }

    #[test]
    fn key_is_injective_on_counts() {
        let a = Shoe::new([1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let b = Shoe::new([0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let c = Shoe::new([0, 0, 0, 0, 0, 0, 0, 0, 0, 128]);
        assert_ne!(a.key(), b.key());
        assert_eq!(c.key(), 128u128 << 72);
        assert_eq!(Shoe::full(8).key() >> 80, 0);
    }

    #[test]
    fn prob() {
        let s = Shoe::full(1);
        assert!((s.prob(Rank::TEN) - 16.0 / 52.0).abs() < 1e-15);
        let total: f64 = Rank::all().map(|r| s.prob(r)).sum();
        assert!((total - 1.0).abs() < 1e-15);
    }
}
