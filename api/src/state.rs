use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use sha2::{Digest, Sha512};
use sqlx::PgPool;

use crate::auth::JwtKeys;
use crate::config::{Config, GoogleConfig};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwt: JwtKeys,
    pub cookie_secure: bool,
    /// A dónde vuelve el navegador después de OAuth.
    pub frontend_origin: String,
    /// `None` si el login con Google está deshabilitado.
    pub google: Option<Arc<Google>>,
    /// Cifra la cookie temporal de OAuth (`state` y `code_verifier`).
    pub cookie_key: Key,
}

/// Cliente OAuth de Google con su cliente HTTP.
pub struct Google {
    pub config: GoogleConfig,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, cfg: &Config) -> AppState {
        let google = cfg.google.clone().map(|config| {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("cliente HTTP");
            Arc::new(Google { config, http })
        });
        // 64 bytes derivados de JWT_SECRET, separados de la clave de los JWT.
        let mut hasher = Sha512::new();
        hasher.update(b"bj-cookie-key:");
        hasher.update(&cfg.jwt_secret);
        let cookie_key = Key::from(&hasher.finalize());
        AppState {
            pool,
            jwt: JwtKeys::new(&cfg.jwt_secret),
            cookie_secure: cfg.cookie_secure,
            frontend_origin: cfg.frontend_origin.trim_end_matches('/').to_string(),
            google,
            cookie_key,
        }
    }
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Key {
        state.cookie_key.clone()
    }
}
