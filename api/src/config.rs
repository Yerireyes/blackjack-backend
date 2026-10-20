use std::net::SocketAddr;

/// Configuración leída de variables de entorno (o de `.env`).
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    /// Clave HMAC de los JWT (al menos 32 bytes).
    pub jwt_secret: Vec<u8>,
    pub bind_addr: SocketAddr,
    /// Origen permitido por CORS (el dominio del front).
    pub frontend_origin: String,
    /// `Secure` en la cookie de sesión (activar en producción con HTTPS).
    pub cookie_secure: bool,
    /// Límite de tasa por IP para `/analyze/*`: una petición cada N ms, ráfaga de M.
    pub analyze_replenish_ms: u64,
    pub analyze_burst: u32,
}

fn var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("falta la variable de entorno {name}"))
}

fn var_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|_| format!("valor inválido en {name}: {v}")),
        Err(_) => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let jwt_secret = var("JWT_SECRET")?.into_bytes();
        if jwt_secret.len() < 32 {
            return Err("JWT_SECRET debe tener al menos 32 caracteres".into());
        }
        Ok(Config {
            database_url: var("DATABASE_URL")?,
            jwt_secret,
            bind_addr: var_or("BIND_ADDR", SocketAddr::from(([127, 0, 0, 1], 8080)))?,
            frontend_origin: var_or("FRONTEND_ORIGIN", "http://localhost:5173".to_string())?,
            cookie_secure: var_or("COOKIE_SECURE", false)?,
            analyze_replenish_ms: var_or("ANALYZE_REPLENISH_MS", 200)?,
            analyze_burst: var_or("ANALYZE_BURST", 20)?,
        })
    }
}
