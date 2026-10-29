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
    /// Proxies cuyo `X-Forwarded-For` se acepta para saber la IP del cliente
    /// (`TRUSTED_PROXIES`, separadas por comas; por defecto solo local).
    pub trusted_proxies: Vec<IpAddr>,
    /// Login con Google; `None` si faltan las variables (queda deshabilitado).
    pub google: Option<GoogleConfig>,
    /// Clave de la semilla del reto diario (`DAILY_SEED_SECRET`). `None`: se deriva de
    /// `JWT_SECRET`.
    pub daily_seed_secret: Option<Vec<u8>>,
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
        Ok(Config {
            database_url: var("DATABASE_URL")?,
            jwt_secret,
            bind_addr: var_or("BIND_ADDR", SocketAddr::from(([127, 0, 0, 1], 8080)))?,
            frontend_origin: var_or("FRONTEND_ORIGIN", "http://localhost:5180".to_string())?,
            cookie_secure: var_or("COOKIE_SECURE", false)?,
            analyze_replenish_ms: var_or("ANALYZE_REPLENISH_MS", 200)?,
            analyze_burst: var_or("ANALYZE_BURST", 20)?,
            auth_burst: var_or("AUTH_BURST", 10)?,
            play_burst: var_or("PLAY_BURST", 10)?,
            trusted_proxies: trusted_proxies()?,
            google: GoogleConfig::from_env()?,
            daily_seed_secret,
        })
    }
}
