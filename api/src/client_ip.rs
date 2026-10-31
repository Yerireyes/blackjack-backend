//! IP del cliente para el límite de tasa, también detrás de un reverse proxy.
//!
//! `X-Forwarded-For` solo se tiene en cuenta si la conexión viene de un proxy de
//! confianza (`TRUSTED_PROXIES`). Caddy y Nginx (`$proxy_add_x_forwarded_for`) agregan
//! la IP que ven al final de lo que mandó el cliente, así que las entradas de la
//! izquierda las controla quien hace la petición: se recorre de derecha a izquierda y
//! se toma la primera que no sea un proxy de confianza.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use axum_extra::extract::cookie::CookieJar;
use ipnet::IpNet;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::GovernorError;
use uuid::Uuid;

use crate::auth::{JwtKeys, COOKIE_NAME};

const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Clave del límite de tasa: la IP real del cliente.
#[derive(Clone, Debug)]
pub struct ClientIpKeyExtractor {
    trusted: Arc<[IpNet]>,
}

impl ClientIpKeyExtractor {
    /// `trusted_proxies`: IPs sueltas (`/32`, `/128`) o redes enteras, como la interna de
    /// una plataforma cuyo proxy no tiene IP fija.
    pub fn new(trusted_proxies: &[IpNet]) -> ClientIpKeyExtractor {
        ClientIpKeyExtractor {
            trusted: trusted_proxies.into(),
        }
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|net| net.contains(&ip))
    }

    /// La IP del cliente a partir de la del par TCP y las cabeceras.
    pub fn client_ip(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let peer = peer.to_canonical();
        if !self.is_trusted(peer) {
            return peer;
        }
        // Varias cabeceras equivalen a una sola con los valores unidos por comas.
        let entries: Vec<&str> = headers
            .get_all(X_FORWARDED_FOR)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .collect();
        for entry in entries.iter().rev() {
            match parse_entry(entry) {
                Some(ip) if self.is_trusted(ip) => continue,
                Some(ip) => return ip,
                // Basura a la derecha del último proxy: no se puede saber más.
                None => break,
            }
        }
        peer
    }
}

/// Una entrada de `X-Forwarded-For`: `1.2.3.4`, `2001:db8::1`, o con puerto
/// (`1.2.3.4:5678`, `[2001:db8::1]:80`).
fn parse_entry(entry: &str) -> Option<IpAddr> {
    let entry = entry.trim();
    entry
        .parse::<IpAddr>()
        .or_else(|_| entry.parse::<SocketAddr>().map(|a| a.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<IpAddr, GovernorError> {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .ok_or(GovernorError::UnableToExtractKey)?
            .ip();
        Ok(self.client_ip(peer, req.headers()))
    }
}

/// Clave del límite de tasa de las rutas de juego: el usuario del JWT de la cookie
/// (sin consultar la base; si la sesión no vale, el handler responde 401) o, sin
/// sesión, la IP.
#[derive(Clone)]
pub struct UserKeyExtractor {
    ip: ClientIpKeyExtractor,
    jwt: JwtKeys,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UserKey {
    User(Uuid),
    Ip(IpAddr),
}

impl UserKeyExtractor {
    pub fn new(ip: ClientIpKeyExtractor, jwt: JwtKeys) -> UserKeyExtractor {
        UserKeyExtractor { ip, jwt }
    }
}

impl KeyExtractor for UserKeyExtractor {
    type Key = UserKey;

    fn extract<T>(&self, req: &Request<T>) -> Result<UserKey, GovernorError> {
        let jar = CookieJar::from_headers(req.headers());
        match jar
            .get(COOKIE_NAME)
            .and_then(|c| self.jwt.verify(c.value()))
        {
            Some(token) => Ok(UserKey::User(token.user_id)),
            None => self.ip.extract(req).map(UserKey::Ip),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(X_FORWARDED_FOR, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn local() -> ClientIpKeyExtractor {
        ClientIpKeyExtractor::new(&[ip("127.0.0.1").into(), ip("::1").into()])
    }

    #[test]
    fn ignores_header_from_untrusted_peer() {
        let h = headers(&["1.2.3.4"]);
        assert_eq!(local().client_ip(ip("203.0.113.9"), &h), ip("203.0.113.9"));
    }

    #[test]
    fn takes_rightmost_untrusted_entry() {
        // El cliente mandó "1.2.3.4" y el proxy agregó la IP real.
        let h = headers(&["1.2.3.4, 203.0.113.9"]);
        assert_eq!(local().client_ip(ip("127.0.0.1"), &h), ip("203.0.113.9"));
        // Lo mismo repartido en dos cabeceras.
        let h = headers(&["1.2.3.4", "203.0.113.9"]);
        assert_eq!(local().client_ip(ip("::1"), &h), ip("203.0.113.9"));
    }

    #[test]
    fn skips_trusted_proxies_in_the_chain() {
        let h = headers(&["1.2.3.4, 203.0.113.9, 127.0.0.1"]);
        assert_eq!(local().client_ip(ip("127.0.0.1"), &h), ip("203.0.113.9"));
    }

    #[test]
    fn falls_back_to_peer() {
        // Sin cabecera (el proxy de Vite no la pone) o con basura a la derecha.
        assert_eq!(
            local().client_ip(ip("127.0.0.1"), &HeaderMap::new()),
            ip("127.0.0.1")
        );
        let h = headers(&["203.0.113.9, nada"]);
        assert_eq!(local().client_ip(ip("127.0.0.1"), &h), ip("127.0.0.1"));
    }

    #[test]
    fn accepts_ports_and_mapped_addresses() {
        let h = headers(&["[2001:db8::1]:443"]);
        assert_eq!(local().client_ip(ip("127.0.0.1"), &h), ip("2001:db8::1"));
        let h = headers(&["203.0.113.9:5000"]);
        assert_eq!(
            local().client_ip(ip("::ffff:127.0.0.1"), &h),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn trusts_nothing_when_list_is_empty() {
        let h = headers(&["1.2.3.4"]);
        let none = ClientIpKeyExtractor::new(&[]);
        assert_eq!(none.client_ip(ip("127.0.0.1"), &h), ip("127.0.0.1"));
    }

    #[test]
    fn trusts_whole_networks() {
        // El proxy de la plataforma conecta desde cualquier IP de su red interna.
        let platform = ClientIpKeyExtractor::new(&["10.0.0.0/8".parse().unwrap()]);
        let h = headers(&["1.2.3.4, 203.0.113.9, 10.1.2.3"]);
        assert_eq!(platform.client_ip(ip("10.200.0.5"), &h), ip("203.0.113.9"));
        assert_eq!(
            platform.client_ip(ip("192.0.2.1"), &h),
            ip("192.0.2.1"),
            "fuera de la red no se confía"
        );
    }
}
