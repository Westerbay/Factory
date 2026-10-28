//! Validate public local configuration before opening sockets. Never load .env.
use std::net::SocketAddr;
use url::{Host, Url};

#[derive(Clone)]
pub struct RuntimeConfig {
    bind: SocketAddr,
    web_origin: String,
    api_origin: String,
}

impl RuntimeConfig {
    pub fn from_values(get: impl Fn(&str) -> Option<String>) -> Result<Self, &'static str> {
        let bind: SocketAddr = get("FACTORY_API_BIND")
            .unwrap_or_else(|| "127.0.0.1:3001".into())
            .parse()
            .map_err(|_| "API_BIND_INVALID")?;
        if !bind.ip().is_loopback() || bind.port() == 0 {
            return Err("API_BIND_NOT_LOCAL");
        }
        let web_origin = local_origin(
            &get("FACTORY_WEB_ORIGIN").unwrap_or_else(|| "http://localhost:3000".into()),
        )?;
        let api = local_origin(
            &get("FACTORY_API_ORIGIN").unwrap_or_else(|| "http://localhost:3001".into()),
        )?;
        let api_url = Url::parse(&api).map_err(|_| "ORIGIN_INVALID")?;
        if api_url.port_or_known_default() != Some(bind.port()) {
            return Err("API_ORIGIN_PORT_MISMATCH");
        }
        Ok(Self {
            bind,
            web_origin,
            api_origin: api,
        })
    }

    pub fn bind(&self) -> SocketAddr {
        self.bind
    }

    pub fn web_origin(&self) -> &str {
        &self.web_origin
    }

    pub fn api_origin(&self) -> &str {
        &self.api_origin
    }
}

fn local_origin(value: &str) -> Result<String, &'static str> {
    let url = Url::parse(value).map_err(|_| "ORIGIN_INVALID")?;
    let local = match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.scheme() != "http"
        || !local
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
        || value != url.origin().ascii_serialization()
    {
        return Err("ORIGIN_NOT_LOCAL_CANONICAL");
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(key: &str, value: &str) -> Result<RuntimeConfig, &'static str> {
        RuntimeConfig::from_values(|name| (name == key).then(|| value.to_owned()))
    }

    #[test]
    fn public_or_ambiguous_origins_are_rejected() {
        for value in [
            "http://example.com",
            "http://localhost.evil.test:3000",
            "http://user:secret@localhost:3000",
            "http://localhost:3000/path",
            "http://localhost:3000?redirect=evil",
            "http://localhost:3000#fragment",
            "http://localhost:3000/",
            "null",
            "https://localhost:3000",
            "http://localhost:0",
        ] {
            assert!(configured("FACTORY_WEB_ORIGIN", value).is_err(), "{value}");
        }
    }

    #[test]
    fn binding_is_loopback_and_matches_the_public_api_port() {
        for value in ["0.0.0.0:3001", "[::]:3001", "127.0.0.1:0", "invalid"] {
            assert!(configured("FACTORY_API_BIND", value).is_err());
        }
        assert!(configured("FACTORY_API_ORIGIN", "http://localhost:9999").is_err());
        let config = RuntimeConfig::from_values(|_| None).unwrap();
        assert_eq!(config.bind(), "127.0.0.1:3001".parse().unwrap());
        assert_eq!(config.web_origin(), "http://localhost:3000");
    }
}
