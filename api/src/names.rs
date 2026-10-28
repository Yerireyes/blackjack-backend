//! Nombres visibles (`display_name`): validación, clave de unicidad y nombres
//! provisionales.

use rand::RngExt;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

pub const MIN_LEN: usize = 3;
pub const MAX_LEN: usize = 20;

/// Se comparan con [`fold`]: sin mayúsculas, tildes, espacios, `_` ni `-`.
const RESERVED: &[&str] = &[
    "admin",
    "administrador",
    "administrator",
    "anonimo",
    "banca",
    "blackjack",
    "blackjackguide",
    "casa",
    "croupier",
    "crupier",
    "dealer",
    "mod",
    "moderador",
    "moderator",
    "null",
    "root",
    "sistema",
    "soporte",
    "staff",
    "support",
    "system",
    "undefined",
];

/// Nombre ya validado y normalizado, con su clave de unicidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayName {
    /// Tal como se muestra (NFC, sin espacios a los lados).
    pub name: String,
    /// Minúsculas Unicode; la base exige que sea única.
    pub key: String,
}

impl DisplayName {
    fn from_valid(name: String) -> DisplayName {
        let key = name.to_lowercase();
        DisplayName { name, key }
    }
}

/// Letras latinas: ASCII más las de Latin-1 (tildes, ñ, ü, ç…). Deja fuera los
/// alfabetos con letras que se confunden con las latinas (cirílico, griego).
fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || (('\u{C0}'..='\u{FF}').contains(&c) && c != '×' && c != '÷')
}

/// Minúsculas, sin tildes ni separadores: "Cása", "c-a s_a" y "CASA" dan "casa".
fn fold(name: &str) -> String {
    name.nfd()
        .filter(|c| !is_combining_mark(*c) && !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Valida un nombre visible: 3–20 caracteres, letras latinas (con tildes y ñ),
/// dígitos, `_`, `-` y espacios internos simples; no reservado.
pub fn validate(raw: &str) -> Result<DisplayName, String> {
    let name: String = raw.trim().nfc().collect();
    let len = name.chars().count();
    if !(MIN_LEN..=MAX_LEN).contains(&len) {
        return Err(format!(
            "el nombre debe tener entre {MIN_LEN} y {MAX_LEN} caracteres"
        ));
    }
    if let Some(c) = name
        .chars()
        .find(|&c| !(is_latin_letter(c) || c.is_ascii_digit() || matches!(c, '_' | '-' | ' ')))
    {
        return Err(format!("el nombre no puede contener «{c}»"));
    }
    if name.contains("  ") {
        return Err("el nombre no puede tener espacios seguidos".into());
    }
    if !name
        .chars()
        .any(|c| is_latin_letter(c) || c.is_ascii_digit())
    {
        return Err("el nombre debe tener al menos una letra o un dígito".into());
    }
    if RESERVED.contains(&fold(&name).as_str()) {
        return Err("ese nombre está reservado".into());
    }
    Ok(DisplayName::from_valid(name))
}

/// Nombre provisional para cuentas creadas con Google (`Jugador4821`). Con
/// `attempt` alto usa más dígitos para que un choque sea improbable.
pub fn provisional(attempt: u32) -> DisplayName {
    let mut rng = rand::rng();
    let n: u32 = if attempt < 5 {
        rng.random_range(1000..10_000)
    } else {
        rng.random_range(100_000..1_000_000)
    };
    DisplayName::from_valid(format!("Jugador{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_normalizes() {
        let n = validate("  Ana_María-7  ").unwrap();
        assert_eq!(n.name, "Ana_María-7");
        assert_eq!(n.key, "ana_maría-7");
        assert!(validate("Juan Pérez").is_ok());
        assert!(validate("007").is_ok());
        // "José" con la tilde como carácter combinado queda en NFC.
        let nfd = validate("Jose\u{301}").unwrap();
        assert_eq!(nfd.name, "José");
        assert_eq!(nfd.key, validate("JOSÉ").unwrap().key);
    }

    #[test]
    fn rejects_invalid() {
        for bad in [
            "ab",
            "a".repeat(21).as_str(),
            "dos  espacios",
            "con.punto",
            "emoji😀",
            "\u{410}dmin", // A cirílica
            "___",
            "x\u{301}\u{301}", // marcas combinadas sueltas
        ] {
            assert!(validate(bad).is_err(), "{bad:?} debería ser inválido");
        }
    }

    #[test]
    fn rejects_reserved_ignoring_case_accents_and_separators() {
        for r in ["admin", "ADMIN", "Cása", "c-a s_a", "Dealer", "Black Jack"] {
            assert_eq!(
                validate(r).unwrap_err(),
                "ese nombre está reservado",
                "{r:?}"
            );
        }
        // Solo coincidencia exacta: un nombre que contiene una palabra reservada vale.
        assert!(validate("Casandra").is_ok());
    }

    #[test]
    fn provisional_names_are_valid() {
        for attempt in [0, 9] {
            let p = provisional(attempt);
            assert!(p.name.starts_with("Jugador"));
            assert_eq!(validate(&p.name).unwrap(), p);
        }
    }
}
