use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

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
    /// Ráfaga del límite por IP de las rutas de cuenta (registro, login, Google,
    /// `PATCH /me`); después, una petición cada 6 s.
    pub auth_burst: u32,
    /// Ráfaga del límite por usuario de las acciones de juego (`/play/*`, crear y
    /// cerrar partidas); después, 5 por segundo.
    pub play_burst: u32,
    /// Salas multijugador abiertas a la vez (`MAX_ROOMS`).
    pub max_rooms: usize,
    /// Proxies cuyo `X-Forwarded-For` se acepta para saber la IP del cliente
    /// (`TRUSTED_PROXIES`, separadas por comas; por defecto solo local).
    pub trusted_proxies: Vec<IpAddr>,
    /// Login con Google; `None` si faltan las variables (queda deshabilitado).
    pub google: Option<GoogleConfig>,
    /// Clave de la semilla del reto diario (`DAILY_SEED_SECRET`). `None`: se deriva de
    /// `JWT_SECRET`.
    pub daily_seed_secret: Option<Vec<u8>>,
    /// Token para `GET /metrics` (`METRICS_TOKEN`). Sin él, las métricas no se sirven.
    pub metrics_token: Option<String>,
    /// Emails de los administradores (`ADMIN_EMAILS`, separados por comas), en minúsculas.
    pub admin_emails: Vec<String>,
}

/// Cliente OAuth de Google. Las URLs de Google se pueden cambiar para los tests.
#[derive(Clone, Debug)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Debe coincidir con la registrada en Google Cloud Console.
    pub redirect_uri: String,
    pub auth_url: String,
    pub token_url: String,
}

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

fn var(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("falta la variable de entorno {name}"))
}

/// La variable si existe y no está vacía.
fn var_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn var_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T, String> {
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|_| format!("valor inválido en {name}: {v}")),
        Err(_) => Ok(default),
    }
}

/// `TRUSTED_PROXIES`: IPs separadas por comas. Sin la variable, 127.0.0.1 y ::1 (el
/// reverse proxy en la misma máquina); vacía, no se confía en nadie.
fn trusted_proxies() -> Result<Vec<IpAddr>, String> {
    let Ok(raw) = std::env::var("TRUSTED_PROXIES") else {
        return Ok(vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]);
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse()
                .map_err(|_| format!("IP inválida en TRUSTED_PROXIES: {s}"))
        })
        .collect()
}

impl GoogleConfig {
    /// Habilitado si están `GOOGLE_CLIENT_ID` y `GOOGLE_CLIENT_SECRET` (entonces
    /// `GOOGLE_REDIRECT_URI` es obligatoria). Solo una de las dos es un error de
    /// configuración, no un "Google deshabilitado".
    fn from_env() -> Result<Option<GoogleConfig>, String> {
        match (var_opt("GOOGLE_CLIENT_ID"), var_opt("GOOGLE_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => Ok(Some(GoogleConfig {
                client_id,
                client_secret,
                redirect_uri: var_opt("GOOGLE_REDIRECT_URI").ok_or(
                    "con GOOGLE_CLIENT_ID y GOOGLE_CLIENT_SECRET hace falta GOOGLE_REDIRECT_URI",
                )?,
                auth_url: GOOGLE_AUTH_URL.into(),
                token_url: GOOGLE_TOKEN_URL.into(),
            })),
            (None, None) => Ok(None),
            _ => Err("para el login con Google hacen falta GOOGLE_CLIENT_ID y \
                      GOOGLE_CLIENT_SECRET (o ninguna de las dos)"
                .into()),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let jwt_secret = var("JWT_SECRET")?.into_bytes();
        if jwt_secret.len() < 32 {
            return Err("JWT_SECRET debe tener al menos 32 caracteres".into());
        }
        let daily_seed_secret = var_opt("DAILY_SEED_SECRET").map(String::into_bytes);
        if daily_seed_secret.as_ref().is_some_and(|s| s.len() < 32) {
            return Err("DAILY_SEED_SECRET debe tener al menos 32 caracteres".into());
        }
        let metrics_token = var_opt("METRICS_TOKEN");
        if metrics_token.as_ref().is_some_and(|t| t.len() < 32) {
            return Err("METRICS_TOKEN debe tener al menos 32 caracteres".into());
        }
        let cfg = Config {
            database_url: var("DATABASE_URL")?,
            jwt_secret,
            bind_addr: var_or("BIND_ADDR", SocketAddr::from(([127, 0, 0, 1], 8080)))?,
            frontend_origin: var_or("FRONTEND_ORIGIN", "http://localhost:5180".to_string())?,
            cookie_secure: var_or("COOKIE_SECURE", false)?,
            analyze_replenish_ms: var_or("ANALYZE_REPLENISH_MS", 200)?,
            analyze_burst: var_or("ANALYZE_BURST", 20)?,
            auth_burst: var_or("AUTH_BURST", 10)?,
            play_burst: var_or("PLAY_BURST", 10)?,
            max_rooms: var_or("MAX_ROOMS", 200)?,
            trusted_proxies: trusted_proxies()?,
            google: GoogleConfig::from_env()?,
            daily_seed_secret,
            metrics_token,
            admin_emails: var_opt("ADMIN_EMAILS")
                .map(|v| {
                    v.split(',')
                        .map(|e| e.trim().to_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        };
        cfg.check_origin()?;
        Ok(cfg)
    }

    /// El origen del front es una URL `http(s)://host[:puerto]` sin ruta, y con HTTPS la
    /// cookie de sesión tiene que ser `Secure` (si no, viajaría también por HTTP).
    pub fn check_origin(&self) -> Result<(), String> {
        let origin = &self.frontend_origin;
        let rest = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"))
            .ok_or_else(|| {
                format!("FRONTEND_ORIGIN debe empezar con http:// o https://: {origin}")
            })?;
        if rest.is_empty() || rest.contains('/') {
            return Err(format!(
                "FRONTEND_ORIGIN es solo el origen, sin ruta ni barra final: {origin}"
            ));
        }
        if origin.starts_with("https://") && !self.cookie_secure {
            return Err(
                "con FRONTEND_ORIGIN en https:// hace falta COOKIE_SECURE=true (producción)".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(origin: &str, secure: bool) -> Config {
        Config {
            database_url: String::new(),
            jwt_secret: vec![0; 32],
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
            frontend_origin: origin.into(),
            cookie_secure: secure,
            analyze_replenish_ms: 200,
            analyze_burst: 20,
            auth_burst: 10,
            play_burst: 10,
            max_rooms: 200,
            trusted_proxies: Vec::new(),
            google: None,
            daily_seed_secret: None,
            metrics_token: None,
            admin_emails: Vec::new(),
        }
    }

    #[test]
    fn https_origin_requires_secure_cookies() {
        assert!(cfg("http://localhost:5180", false).check_origin().is_ok());
        assert!(cfg("https://blackjack.example", true)
            .check_origin()
            .is_ok());
        assert!(cfg("https://blackjack.example", false)
            .check_origin()
            .is_err());
        assert!(cfg("https://blackjack.example/", true)
            .check_origin()
            .is_err());
        assert!(cfg("blackjack.example", true).check_origin().is_err());
    }
}
