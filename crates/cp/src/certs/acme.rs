// SPDX-License-Identifier: AGPL-3.0-only
//! Obtaining and renewing certificates from an ACME server with the HTTP-01 challenge.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};

use super::resolver::{CertResolver, certified_key};
use crate::{config::Config, secrets::write_private};

/// HTTP-01 answers waiting to be fetched by the ACME server: token to key authorisation.
#[derive(Clone, Default)]
pub struct ChallengeMap(Arc<RwLock<HashMap<String, String>>>);

impl ChallengeMap {
    pub fn get(&self, token: &str) -> Option<String> {
        self.0.read().expect("challenge map lock").get(token).cloned()
    }
    fn insert(&self, token: String, key_authorization: String) {
        self.0.write().expect("challenge map lock").insert(token, key_authorization);
    }
    fn remove(&self, token: &str) {
        self.0.write().expect("challenge map lock").remove(token);
    }
}

const RENEW_BEFORE: chrono::Duration = chrono::Duration::days(30);
const CHECK_EVERY: Duration = Duration::from_secs(12 * 3600);

pub struct AcmeIssuer {
    directory: String,
    email: Option<String>,
    root_ca: Option<PathBuf>,
    dir: PathBuf,
    challenges: ChallengeMap,
}

impl AcmeIssuer {
    pub fn new(config: &Config, challenges: ChallengeMap) -> anyhow::Result<Self> {
        let dir = config.data_dir.join("acme");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self {
            directory: config.acme_directory.clone(),
            email: config.acme_email.clone(),
            root_ca: config.acme_root_ca.clone(),
            dir,
            challenges,
        })
    }

    fn cert_path(&self, domain: &str) -> PathBuf {
        self.dir.join(format!("{domain}.crt"))
    }
    fn key_path(&self, domain: &str) -> PathBuf {
        self.dir.join(format!("{domain}.key"))
    }

    async fn account(&self) -> anyhow::Result<Account> {
        let builder = || match &self.root_ca {
            Some(root) => Account::builder_with_root(root),
            None => Account::builder(),
        };
        let path = self.dir.join("account.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let credentials: AccountCredentials = serde_json::from_str(&text).context("reading ACME account credentials")?;
            return Ok(builder()?.from_credentials(credentials).await?);
        }
        let contact: Vec<String> = self.email.iter().map(|e| format!("mailto:{e}")).collect();
        let contact: Vec<&str> = contact.iter().map(String::as_str).collect();
        let (account, credentials) = builder()?
            .create(
                &NewAccount { contact: &contact, terms_of_service_agreed: true, only_return_existing: false },
                self.directory.clone(),
                None,
            )
            .await
            .context("creating the ACME account")?;
        write_private(&path, serde_json::to_string(&credentials)?.as_bytes())?;
        Ok(account)
    }

    /// Runs one full order for `domain` and returns `(certificate chain PEM, private key PEM)`.
    pub async fn issue(&self, domain: &str) -> anyhow::Result<(String, String)> {
        let account = self.account().await?;
        let mut order = account.new_order(&NewOrder::new(&[Identifier::Dns(domain.to_owned())])).await.context("creating the order")?;
        let mut tokens = Vec::new();
        let mut authorizations = order.authorizations();
        while let Some(result) = authorizations.next().await {
            let mut authz = result?;
            match authz.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                other => bail!("authorisation is {other:?}"),
            }
            let mut challenge = authz.challenge(ChallengeType::Http01).context("the ACME server offered no http-01 challenge")?;
            let token = challenge.token.clone();
            self.challenges.insert(token.clone(), challenge.key_authorization().as_str().to_owned());
            tokens.push(token);
            challenge.set_ready().await?;
        }
        let outcome = async {
            let status = order.poll_ready(&RetryPolicy::default()).await?;
            if status != OrderStatus::Ready {
                bail!("order ended as {status:?}; check that {domain} points at this server and port 80 is reachable");
            }
            let key_pem = order.finalize().await?;
            let chain = order.poll_certificate(&RetryPolicy::default()).await?;
            Ok::<_, anyhow::Error>((chain, key_pem))
        }
        .await;
        tokens.iter().for_each(|t| self.challenges.remove(t));
        outcome
    }

    fn load_saved(&self, domain: &str) -> Option<(String, String)> {
        Some((std::fs::read_to_string(self.cert_path(domain)).ok()?, std::fs::read_to_string(self.key_path(domain)).ok()?))
    }

    fn save(&self, domain: &str, chain: &str, key: &str) -> anyhow::Result<()> {
        // Replace atomically so a crash never leaves a certificate without its key.
        let (cert_tmp, key_tmp) = (self.dir.join(format!("{domain}.crt.new")), self.dir.join(format!("{domain}.key.new")));
        let _ = std::fs::remove_file(&cert_tmp);
        let _ = std::fs::remove_file(&key_tmp);
        write_private(&key_tmp, key.as_bytes())?;
        write_private(&cert_tmp, chain.as_bytes())?;
        std::fs::rename(&key_tmp, self.key_path(domain))?;
        std::fs::rename(&cert_tmp, self.cert_path(domain))?;
        Ok(())
    }
}

/// Expiry of the first certificate in a PEM chain.
pub fn not_after(chain_pem: &str) -> anyhow::Result<DateTime<Utc>> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(chain_pem.as_bytes()).context("parsing certificate PEM")?;
    let cert = pem.parse_x509().context("parsing certificate")?;
    DateTime::from_timestamp(cert.validity().not_after.timestamp(), 0).context("certificate expiry out of range")
}

/// Keeps a valid certificate for `domain` in the resolver for the life of the process.
pub async fn manage(issuer: AcmeIssuer, resolver: Arc<CertResolver>, domain: String) {
    let mut backoff = Duration::from_secs(30);
    loop {
        match ensure(&issuer, &resolver, &domain).await {
            Ok(()) => {
                backoff = Duration::from_secs(30);
                tokio::time::sleep(CHECK_EVERY).await;
            }
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), %domain, retry_in = ?backoff, "certificate not available yet");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(3600));
            }
        }
    }
}

async fn ensure(issuer: &AcmeIssuer, resolver: &CertResolver, domain: &str) -> anyhow::Result<()> {
    if let Some((chain, key)) = issuer.load_saved(domain)
        && let Ok(expiry) = not_after(&chain)
    {
        if !resolver.has(domain) {
            resolver.set(domain, Arc::new(certified_key(&chain, &key)?));
        }
        if expiry - Utc::now() > RENEW_BEFORE {
            return Ok(());
        }
        tracing::info!(%domain, %expiry, "certificate is close to expiry, renewing");
    }
    let (chain, key) = issuer.issue(domain).await?;
    let ck = certified_key(&chain, &key)?;
    issuer.save(domain, &chain, &key)?;
    resolver.set(domain, Arc::new(ck));
    tracing::info!(%domain, expiry = %not_after(&chain)?, "certificate installed");
    Ok(())
}
