//! Juego en el servidor (Casual y Ranked): zapato con orden real, reparto,
//! acciones, turno del dealer y liquidación. Puro, sin IO ni fuente de azar propia
//! (el barajado recibe el generador), para que se pueda probar sin base de datos.

pub mod card;
pub mod contract;
pub mod round;
pub mod shoe;

pub use card::{Card, Face, Suit};
pub use contract::*;
pub use round::{PlayerHand, Round};
pub use shoe::GameShoe;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GameError {
    #[error("esa acción no está permitida ahora")]
    ActionNotAllowed,
    /// No debería pasar: se rebaraja antes de cada ronda con margen para la peor.
    #[error("se acabaron las cartas del zapato")]
    ShoeEmpty,
}

#[cfg(test)]
mod tests;
