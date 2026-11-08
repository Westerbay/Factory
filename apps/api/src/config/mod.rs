//! Validate runtime configuration before opening sockets.
mod local;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use url::{Host, Url};

#[derive(Clone)]
pub struct RuntimeConfig {
    bind: SocketAddr,
    web_origin: String,
    api_origin: String,
    workspace: PathBuf,
    identity: Option<IdentityConfig>,
    production_database_password: Option<String>,
}

// Never derive Debug: the provider secret must stay server-side.
#[derive(Clone)]
pub struct IdentityConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl RuntimeConfig {
    pub fn from_values(get: impl Fn(&str) -> Option<String>) -> Result<Self, &'static str> {
        let production = match get("FACTORY_DEPLOYMENT").as_deref() {
            None | Some("local") => false,
            Some("production") => true,
            Some(_) => return Err("DEPLOYMENT_INVALID"),
        };
        let default_bind = if production {
            "0.0.0.0:3001"
        } else {
            "127.0.0.1:3001"
        };
        let bind: SocketAddr = get("FACTORY_API_BIND")
            .unwrap_or_else(|| default_bind.into())
            .parse()
            .map_err(|_| "API_BIND_INVALID")?;
        if bind.port() == 0 {
            return Err("API_BIND_INVALID");
        }
        let (web_origin, api_origin, production_database_password) = if production {
            let (web, api) = production_origins(&get)?;
            let password = get("FACTORY_DB_PASSWORD")
                .filter(|value| !value.trim().is_empty() && value != "local-fixture-only")
                .ok_or("PRODUCTION_DATABASE_PASSWORD_REQUIRED")?;
            (web, api, Some(password))
        } else {
            let (web, api) = local_origins(&get, bind)?;
            (web, api, None)
        };
        let workspace = match get("FACTORY_WORKSPACE_ROOT") {
            Some(value) => PathBuf::from(value),
            None => std::env::current_dir().map_err(|_| "WORKSPACE_UNAVAILABLE")?,
        };
        if !workspace.is_absolute() {
            return Err("WORKSPACE_MUST_BE_ABSOLUTE");
        }
        let identity = match (
            get("FACTORY_GITHUB_CLIENT_ID"),
            get("FACTORY_GITHUB_CLIENT_SECRET"),
        ) {
            (Some(client_id), Some(client_secret))
                if !client_id.trim().is_empty() && !client_secret.trim().is_empty() =>
            {
                Some(IdentityConfig {
                    client_id,
                    client_secret,
                })
            }
            _ => None,
        };
        if production && identity.is_none() {
            return Err("IDENTITY_CONFIGURATION_REQUIRED");
        }
        Ok(Self {
            bind,
            web_origin,
            api_origin,
            workspace,
            identity,
            production_database_password,
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

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    pub fn identity(&self) -> Option<&IdentityConfig> {
        self.identity.as_ref()
    }
    pub(crate) fn production_database_password(&self) -> Option<&str> {
        self.production_database_password.as_deref()
    }
}

fn production_origins(
    get: &impl Fn(&str) -> Option<String>,
) -> Result<(String, String), &'static str> {
    let web = get("FACTORY_WEB_ORIGIN").filter(|origin| !origin.is_empty());
    let api = get("FACTORY_API_ORIGIN").filter(|origin| !origin.is_empty());
    let (web, api) = match web {
        Some(web) => {
            let api = api.unwrap_or_else(|| web.clone());
            (web, api)
        }
        None => {
            let domain = get("FACTORY_DOMAIN")
                .filter(|domain| !domain.is_empty())
                .ok_or("PRODUCTION_WEB_ORIGIN_REQUIRED")?;
            let web = format!("https://app.{domain}");
            let legacy_api = format!("https://api.{domain}");
            (web, api.unwrap_or(legacy_api))
        }
    };
    let web_parent = production_parent(&web).ok_or("PRODUCTION_WEB_ORIGIN_INVALID")?;
    let api_parent = production_parent(&api).ok_or("PRODUCTION_API_ORIGIN_INVALID")?;
    if web_parent != api_parent {
        return Err("PRODUCTION_ORIGINS_MUST_SHARE_DOMAIN");
    }
    Ok((web, api))
}

/// Require a canonical HTTPS origin on a subdomain of an operator-owned domain.
fn production_parent(origin: &str) -> Option<&str> {
    let host = origin.strip_prefix("https://")?;
    let (_, parent) = host.split_once('.')?;
    let suffix = parent.rsplit('.').next()?;
    let valid = host.len() <= 253
        && parent.contains('.')
        && suffix != "localhost"
        && suffix.bytes().any(|byte| byte.is_ascii_lowercase())
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        });
    valid.then_some(parent)
}

fn local_origins(
    get: impl Fn(&str) -> Option<String>,
    bind: SocketAddr,
) -> Result<(String, String), &'static str> {
    if !bind.ip().is_loopback() {
        return Err("API_BIND_NOT_LOCAL");
    }
    let web =
        local_origin(&get("FACTORY_WEB_ORIGIN").unwrap_or_else(|| "http://localhost:3000".into()))?;
    let api =
        local_origin(&get("FACTORY_API_ORIGIN").unwrap_or_else(|| "http://localhost:3001".into()))?;
    let api_url = Url::parse(&api).map_err(|_| "ORIGIN_INVALID")?;
    if api_url.port_or_known_default() != Some(bind.port()) {
        return Err("API_ORIGIN_PORT_MISMATCH");
    }
    let web_url = Url::parse(&web).map_err(|_| "ORIGIN_INVALID")?;
    if web_url.host() != api_url.host() {
        return Err("ORIGINS_MUST_SHARE_LOCAL_HOST");
    }
    Ok((web, api))
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

    #[test]
    fn local_cookies_require_matching_browser_hosts() {
        assert!(configured("FACTORY_WEB_ORIGIN", "http://127.0.0.1:3000").is_err());
        assert!(configured("FACTORY_WORKSPACE_ROOT", "relative/path").is_err());
    }

    fn production(overrides: &[(&str, Option<&str>)]) -> Result<RuntimeConfig, &'static str> {
        RuntimeConfig::from_values(|name| {
            if let Some((_, value)) = overrides.iter().find(|(key, _)| *key == name) {
                return value.map(str::to_owned);
            }
            match name {
                "FACTORY_DEPLOYMENT" => Some("production".into()),
                "FACTORY_DOMAIN" => Some("example.com".into()),
                "FACTORY_DB_PASSWORD" => Some("synthetic-password".into()),
                "FACTORY_GITHUB_CLIENT_ID" => Some("synthetic-client".into()),
                "FACTORY_GITHUB_CLIENT_SECRET" => Some("synthetic-secret".into()),
                _ => None,
            }
        })
    }

    #[test]
    fn production_derives_https_sibling_origins_and_requires_credentials() {
        let config = production(&[]).unwrap();
        assert_eq!(config.bind(), "0.0.0.0:3001".parse().unwrap());
        assert_eq!(config.web_origin(), "https://app.example.com");
        assert_eq!(config.api_origin(), "https://api.example.com");
        for domain in [
            "example.com/path",
            "https://example.com",
            "example.com:443",
            "a..com",
            "127.0.0.1",
            "app.localhost",
        ] {
            assert!(matches!(
                production(&[("FACTORY_DOMAIN", Some(domain))]),
                Err("PRODUCTION_WEB_ORIGIN_INVALID")
            ));
        }
        assert!(matches!(
            production(&[("FACTORY_DB_PASSWORD", None)]),
            Err("PRODUCTION_DATABASE_PASSWORD_REQUIRED")
        ));
        assert!(matches!(
            production(&[("FACTORY_GITHUB_CLIENT_SECRET", None)]),
            Err("IDENTITY_CONFIGURATION_REQUIRED")
        ));
    }

    #[test]
    fn production_web_origin_defaults_to_serving_the_api_on_the_same_host() {
        let config = production(&[
            ("FACTORY_DOMAIN", None),
            ("FACTORY_WEB_ORIGIN", Some("https://factory.example.com")),
        ])
        .unwrap();
        assert_eq!(config.web_origin(), "https://factory.example.com");
        assert_eq!(config.api_origin(), config.web_origin());
    }

    #[test]
    fn production_api_origin_accepts_only_https_on_the_same_or_a_sibling_host() {
        let web = ("FACTORY_WEB_ORIGIN", Some("https://factory.example.com"));
        let configured_api = "https://factory-api.example.com";
        let config = production(&[web, ("FACTORY_API_ORIGIN", Some(configured_api))]).unwrap();
        assert_eq!(config.api_origin(), configured_api);
        assert!(matches!(
            production(&[web, ("FACTORY_API_ORIGIN", Some("https://api.other.com"))]),
            Err("PRODUCTION_ORIGINS_MUST_SHARE_DOMAIN")
        ));
        for invalid in [
            "http://factory-api.example.com",
            "https://factory-api.example.com:443",
            "https://factory-api.example.com/path",
            "https://name@factory-api.example.com",
        ] {
            assert!(matches!(
                production(&[web, ("FACTORY_API_ORIGIN", Some(invalid))]),
                Err("PRODUCTION_API_ORIGIN_INVALID")
            ));
        }
    }

    #[test]
    fn oauth_configuration_does_not_require_an_admin_environment_variable() {
        let config = RuntimeConfig::from_values(|key| match key {
            "FACTORY_GITHUB_CLIENT_ID" => Some("synthetic-client".into()),
            "FACTORY_GITHUB_CLIENT_SECRET" => Some("synthetic-secret".into()),
            _ => None,
        })
        .unwrap();
        assert!(config.identity().is_some());
        for secret in [None, Some(""), Some(" ")] {
            let config = RuntimeConfig::from_values(|key| match key {
                "FACTORY_GITHUB_CLIENT_ID" => Some("synthetic-client".into()),
                "FACTORY_GITHUB_CLIENT_SECRET" => secret.map(str::to_owned),
                _ => None,
            })
            .unwrap();
            assert!(config.identity().is_none());
        }
    }
}
