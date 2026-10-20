use sqlx::PgPool;

use crate::auth::JwtKeys;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwt: JwtKeys,
    pub cookie_secure: bool,
}
