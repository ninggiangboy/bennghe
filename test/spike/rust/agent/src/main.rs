//! Minimal agent for the RAM spike: WSS to a fake control plane (protobuf frames) and a
//! Docker `info` + `list services` poll every 15 seconds, on a current-thread runtime.
use std::{env, fs, sync::Arc, time::Duration};

use bollard::Docker;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message, Connector};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Clone, PartialEq, prost::Message)]
struct Report {
    #[prost(string, tag = "1")]
    engine_version: String,
    #[prost(uint32, tag = "2")]
    services: u32,
    #[prost(string, repeated, tag = "3")]
    names: Vec<String>,
}

fn tls_config(ca_path: &str) -> Arc<rustls::ClientConfig> {
    let pem = fs::read(ca_path).expect("read CA");
    let mut roots = rustls::RootCertStore::empty();
    for line_block in std::str::from_utf8(&pem).unwrap().split("-----END CERTIFICATE-----") {
        if let Some(b64) = line_block.split("-----BEGIN CERTIFICATE-----").nth(1) {
            let b64: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
            roots.add(decode_b64(&b64).into()).unwrap();
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn decode_b64(s: &str) -> Vec<u8> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0);
    for c in s.bytes().filter(|c| *c != b'=') {
        buf = (buf << 6) | T.iter().position(|t| *t == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let url = env::var("SPIKE_URL").unwrap_or_else(|_| "wss://localhost:8443/".into());
    let ca = env::var("SPIKE_CA").unwrap_or_else(|_| "/spike/ca.pem".into());
    let docker = Docker::connect_with_unix_defaults().expect("docker");
    let (mut ws, _) = connect_async_tls_with_config(&url, None, false, Some(Connector::Rustls(tls_config(&ca))))
        .await
        .expect("ws connect");
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let info = docker.info().await.expect("info");
                let svcs = docker.list_services(None).await.expect("services");
                let report = Report {
                    engine_version: info.server_version.unwrap_or_default(),
                    services: svcs.len() as u32,
                    names: svcs.iter().filter_map(|s| s.spec.as_ref().and_then(|p| p.name.clone())).collect(),
                };
                ws.send(Message::Binary(report.encode_to_vec().into())).await.expect("send");
            }
            msg = ws.next() => { if msg.is_none() { return; } }
        }
    }
}
