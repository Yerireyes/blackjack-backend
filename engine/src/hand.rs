use crate::rank::Rank;

/// Mano representada por su suma dura (As = 1) y si contiene algún As.
/// Es todo lo que la recursión necesita: el total y si es suave se derivan de ahí.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hand {
    pub hard: u8,
    pub has_ace: bool,
    pub n_cards: u8,
}

impl Hand {
    pub const EMPTY: Hand = Hand {
        hard: 0,
        has_ace: false,
        n_cards: 0,
    };

    pub fn from_cards(cards: &[Rank]) -> Hand {
        cards.iter().fold(Hand::EMPTY, |h, &r| h.with_card(r))
    }

    pub fn with_card(self, r: Rank) -> Hand {
        Hand {
            hard: self.hard + r.value(),
            has_ace: self.has_ace || r.is_ace(),
            n_cards: self.n_cards + 1,
        }
    }

    pub fn is_soft(self) -> bool {
        self.has_ace && self.hard + 10 <= 21
    }

    /// Mejor total (un As cuenta 11 si no se pasa).
    pub fn total(self) -> u8 {
        if self.is_soft() {
            self.hard + 10
        } else {
            self.hard
        }
    }

    pub fn is_bust(self) -> bool {
        self.hard > 21
    }

    /// Dos cartas que suman 21. Solo es blackjack natural si no viene de dividir.
    pub fn is_two_card_21(self) -> bool {
        self.n_cards == 2 && self.total() == 21
    }
}

/// Descripción completa de la mano del jugador para decidir qué acciones existen.
#[derive(Clone, Copy, Debug)]
pub struct HandState {
    pub total: u8,
    pub soft: bool,
    pub n_cards: u8,
    pub is_pair: Option<Rank>,
    pub from_split: bool,
}

impl HandState {
    pub fn new(cards: &[Rank], from_split: bool) -> HandState {
        let h = Hand::from_cards(cards);
        let is_pair = match cards {
            [a, b] if a == b => Some(*a),
            _ => None,
        };
        HandState {
            total: h.total(),
            soft: h.is_soft(),
            n_cards: h.n_cards,
            is_pair,
            from_split,
        }
    }

    pub fn is_natural(&self) -> bool {
        !self.from_split && self.n_cards == 2 && self.total == 21
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(i: u8) -> Rank {
        Rank::new(i).unwrap()
    }

    #[test]
    fn totals() {
        let h = Hand::from_cards(&[Rank::ACE, r(5)]); // A,6
        assert_eq!(h.total(), 17);
        assert!(h.is_soft());
        let h = h.with_card(Rank::TEN);
        assert_eq!(h.total(), 17);
        assert!(!h.is_soft());
        let bj = Hand::from_cards(&[Rank::ACE, Rank::TEN]);
        assert!(bj.is_two_card_21());
        let aa = Hand::from_cards(&[Rank::ACE, Rank::ACE]);
        assert_eq!(aa.total(), 12);
        assert!(aa.is_soft());
        assert!(Hand::from_cards(&[Rank::TEN, Rank::TEN, r(1)]).is_bust());
    }

    #[test]
    fn state() {
        let s = HandState::new(&[r(7), r(7)], false);
        assert_eq!(s.is_pair, Some(r(7)));
        assert_eq!(s.total, 16);
        assert!(HandState::new(&[Rank::ACE, Rank::TEN], false).is_natural());
        assert!(!HandState::new(&[Rank::ACE, Rank::TEN], true).is_natural());
    }
}
