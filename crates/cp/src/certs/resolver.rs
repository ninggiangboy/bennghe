// SPDX-License-Identifier: AGPL-3.0-only
//! Certificate lookup by SNI, replaceable at runtime without restarting listeners.
use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, bail};
use arc_swap::ArcSwap;
use rustls::{
    crypto::ring::sign::any_supported_type,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};

#[derive(Debug, Default)]
pub struct CertResolver {
    certs: ArcSwap<HashMap<String, Arc<CertifiedKey>>>,
}

impl CertResolver {
    /// Installs or replaces the certificate served for `domain`. New connections use it at once.
    pub fn set(&self, domain: &str, key: Arc<CertifiedKey>) {
        let domain = domain.to_ascii_lowercase();
        self.certs.rcu(|current| {
            let mut next = HashMap::clone(current);
            next.insert(domain.clone(), key.clone());
            next
        });
    }

    pub fn has(&self, domain: &str) -> bool {
        self.certs.load().contains_key(&domain.to_ascii_lowercase())
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let name = hello.server_name()?.to_ascii_lowercase();
        self.certs.load().get(&name).cloned()
    }
}

/// Builds a servable key from PEM text.
pub fn certified_key(chain_pem: &str, key_pem: &str) -> anyhow::Result<CertifiedKey> {
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(chain_pem.as_bytes()).collect::<Result<_, _>>().context("reading certificate chain")?;
    if chain.is_empty() {
        bail!("certificate chain is empty");
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).context("reading private key")?;
    let signing = any_supported_type(&key).context("unsupported private key type")?;
    Ok(CertifiedKey::new(chain, signing))
}
