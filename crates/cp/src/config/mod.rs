// SPDX-License-Identifier: AGPL-3.0-only
//! Configuration: an optional TOML file (`BN_CONFIG`) overridden by `BN_*` environment variables.
use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};
use ipnet::IpNet;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// The control plane terminates TLS itself and obtains its certificate over ACME HTTP-01.
    Acme,
    /// Plain HTTP only, for deployments behind a reverse proxy that terminates TLS.
    Off,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Holds `master.key` and `signing.key`; must be readable only by the control plane user.
    pub data_dir: PathBuf,
    pub http_addr: SocketAddr,
    pub https_addr: SocketAddr,
    pub tls_mode: TlsMode,
    pub public_domain: Option<String>,
    pub acme_email: Option<String>,
    pub acme_directory: String,
    /// PEM file with the root CA of a private or test ACME server. Unset for public CAs.
    pub acme_root_ca: Option<PathBuf>,
    /// Peers whose `X-Forwarded-For` header is believed.
    pub trusted_proxies: Vec<IpNet>,
    /// Drops the `__Host-` cookie prefix and the `Secure` flag. Local development over HTTP only.
    pub insecure_cookies: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    database_url: Option<String>,
    data_dir: Option<PathBuf>,
    http_addr: Option<SocketAddr>,
    https_addr: Option<SocketAddr>,
    tls_mode: Option<TlsMode>,
    public_domain: Option<String>,
    acme_email: Option<String>,
    acme_directory: Option<String>,
    acme_root_ca: Option<PathBuf>,
    trusted_proxies: Option<Vec<IpNet>>,
    insecure_cookies: Option<bool>,
}

const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let file = match std::env::var("BN_CONFIG") {
            Ok(path) => {
                let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
                toml::from_str(&text).with_context(|| format!("parsing {path}"))?
            }
            Err(_) => FileConfig::default(),
        };
        Self::from_sources(file, |k| std::env::var(k).ok())
    }

    fn from_sources(file: FileConfig, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        fn parse<T: std::str::FromStr>(env: &impl Fn(&str) -> Option<String>, key: &str) -> anyhow::Result<Option<T>>
        where
            T::Err: std::fmt::Display,
        {
            env(key).map(|v| v.parse::<T>().map_err(|e| anyhow::anyhow!("{key}: {e}"))).transpose()
        }

        let database_url =
            env("BN_DATABASE_URL").or(file.database_url).context("BN_DATABASE_URL (or database_url in the config file) is required")?;
        let tls_mode = match env("BN_TLS_MODE").as_deref() {
            Some("acme") => TlsMode::Acme,
            Some("off") => TlsMode::Off,
            Some(other) => bail!("BN_TLS_MODE must be `acme` or `off`, got `{other}`"),
            None => file.tls_mode.unwrap_or(TlsMode::Acme),
        };
        let trusted_proxies = match env("BN_TRUSTED_PROXIES") {
            Some(list) => list
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<IpNet>().with_context(|| format!("BN_TRUSTED_PROXIES entry `{s}`")))
                .collect::<anyhow::Result<Vec<_>>>()?,
            None => file.trusted_proxies.unwrap_or_default(),
        };
        let config = Self {
            database_url,
            data_dir: env("BN_DATA_DIR").map(PathBuf::from).or(file.data_dir).unwrap_or_else(|| "/var/lib/bennghe".into()),
            http_addr: parse(&env, "BN_HTTP_ADDR")?.or(file.http_addr).unwrap_or_else(|| ([0, 0, 0, 0], 80).into()),
            https_addr: parse(&env, "BN_HTTPS_ADDR")?.or(file.https_addr).unwrap_or_else(|| ([0, 0, 0, 0], 443).into()),
            tls_mode,
            public_domain: env("BN_PUBLIC_DOMAIN").or(file.public_domain),
            acme_email: env("BN_ACME_EMAIL").or(file.acme_email),
            acme_directory: env("BN_ACME_DIRECTORY").or(file.acme_directory).unwrap_or_else(|| LETS_ENCRYPT.into()),
            acme_root_ca: env("BN_ACME_ROOT_CA").map(PathBuf::from).or(file.acme_root_ca),
            trusted_proxies,
            insecure_cookies: parse::<bool>(&env, "BN_INSECURE_COOKIES")?.or(file.insecure_cookies).unwrap_or(false),
        };
        if config.tls_mode == TlsMode::Acme && config.public_domain.is_none() {
            bail!("BN_PUBLIC_DOMAIN is required when BN_TLS_MODE=acme");
        }
        Ok(config)
    }

    pub fn master_key_path(&self) -> PathBuf {
        self.data_dir.join("master.key")
    }

    pub fn signing_key_path(&self) -> PathBuf {
        self.data_dir.join("signing.key")
    }

    /// Settings for tests and tools that do not read the environment.
    pub fn for_tests(data_dir: PathBuf) -> Self {
        Self {
            database_url: String::new(),
            data_dir,
            http_addr: ([127, 0, 0, 1], 0).into(),
            https_addr: ([127, 0, 0, 1], 0).into(),
            tls_mode: TlsMode::Off,
            public_domain: None,
            acme_email: None,
            acme_directory: LETS_ENCRYPT.into(),
            acme_root_ca: None,
            trusted_proxies: Vec::new(),
            insecure_cookies: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn env_overrides_file() {
        let file = FileConfig { database_url: Some("postgres://file".into()), tls_mode: Some(TlsMode::Off), ..Default::default() };
        let cfg = Config::from_sources(file, env_of(&[("BN_DATABASE_URL", "postgres://env")])).unwrap();
        assert_eq!(cfg.database_url, "postgres://env");
        assert_eq!(cfg.tls_mode, TlsMode::Off);
    }

    #[test]
    fn acme_requires_a_domain() {
        let err = Config::from_sources(FileConfig::default(), env_of(&[("BN_DATABASE_URL", "x")])).unwrap_err();
        assert!(err.to_string().contains("BN_PUBLIC_DOMAIN"));
    }

    #[test]
    fn rejects_bad_proxy_cidr() {
        let env = env_of(&[("BN_DATABASE_URL", "x"), ("BN_TLS_MODE", "off"), ("BN_TRUSTED_PROXIES", "10.0.0.0/8,nope")]);
        assert!(Config::from_sources(FileConfig::default(), env).is_err());
    }
}
