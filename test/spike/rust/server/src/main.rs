//! Fake control-plane endpoint for the memory spike: accepts WSS, acks every binary frame.
use std::{env, fs, sync::Arc};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_rustls::{rustls, TlsAcceptor};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() {
    let dir = env::args().nth(1).unwrap_or_else(|| "/spike".into());
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    fs::write(format!("{dir}/ca.pem"), ck.cert.pem()).unwrap();
    let cert = ck.cert.der().clone();
    let key = rustls::pki_types::PrivateKeyDer::try_from(ck.signing_key.serialize_der()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:8443").await.unwrap();
    println!("listening on 127.0.0.1:8443, CA in {dir}/ca.pem");
    loop {
        let (tcp, _) = listener.accept().await.unwrap();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else { return };
            let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else { return };
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Binary(b) = msg {
                    println!("recv {} bytes", b.len());
                    if ws.send(Message::Binary(vec![1].into())).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
}
