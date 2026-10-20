use thiserror::Error;

/// Errores de validación de entrada. Los mensajes van tal cual al front.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EngineError {
    #[error("reglas inválidas: {0}")]
    InvalidRules(String),
    #[error("zapato inválido: {0}")]
    InvalidShoe(String),
    #[error("mano inválida: {0}")]
    InvalidHand(String),
}
