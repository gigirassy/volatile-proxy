use std::{
    net::SocketAddr,
    println,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering::SeqCst},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use arti::socks::run_socks_proxy;
use arti_client::{TorClient, TorClientConfig};
use arti_hyper::*;
use futures::future::join_all;
use hyper::Body;
use tls_api::{TlsConnector as TlsConnectorTrait, TlsConnectorBuilder};
use tokio::{
    net::{TcpListener, TcpStream},
    time::sleep,
};
use tor_rtcompat::{BlockOn, Runtime};

// https://gitlab.torproject.org/tpo/core/arti/-/issues/715
#[cfg(not(target_vendor = "apple"))]
use tls_api_native_tls::TlsConnector;
#[cfg(target_vendor = "apple")]
use tls_api_openssl::TlsConnector;

#[derive(Debug)]
struct Proxy {
    ready: Arc<AtomicBool>,
    port: u16,
}

fn main() {
    let runtime = tor_rtcompat::tokio::TokioNativeTlsRuntime::create().unwrap();

    runtime.clone().block_on(run(runtime));
}

async fn run<R: Runtime>(runtime: R) {
    let config = TorClientConfig::default();

    let tor_client = TorClient::with_runtime(runtime.clone())
        .config(config)
        .bootstrap_behavior(arti_client::BootstrapBehavior::OnDemand)
        .create_unbootstrapped()
        .unwrap();

    let mut ready_proxies: Vec<Proxy> = Default::default();

    for i in 0..4 {
        let ready: Arc<AtomicBool> = Default::default();
        let port = 9051 + i;
        ready_proxies.push(Proxy {
            ready: ready.clone(),
            port: port,
        });
        tokio::spawn(run_proxy(
            runtime.clone(),
            tor_client.isolated_client(),
            ready,
            port,
        ));
    }

    let listener = TcpListener::bind("127.0.0.1:9050").await.unwrap();
    while let Ok((mut ingress, _)) = listener.accept().await {
        ready_proxies
            .iter()
            .filter(|proxy| proxy.ready.load(SeqCst))
            .for_each(|proxy| println!("{:#?}", proxy));

        tokio::spawn(async move {
            let mut egress = TcpStream::connect("127.0.0.1:9051").await.unwrap();
            match tokio::io::copy_bidirectional(&mut ingress, &mut egress).await {
                Ok((to_egress, to_ingress)) => {
                    println!(
                        "Connection ended gracefully ({} bytes from client, {} bytes from server)",
                        to_egress, to_ingress
                    );
                }
                Err(err) => {
                    println!("Error while proxying: {}", err);
                }
            }
        });
    }
}

/// Run main proxy loop
async fn run_proxy<R: Runtime>(
    runtime: R,
    mut tor_client: TorClient<R>,
    ready: Arc<AtomicBool>,
    socks_port: u16,
) -> Result<()> {
    tor_client = tor_client.isolated_client();

    let tls_connector = TlsConnector::builder()?.build()?;

    let tor_connector = ArtiHttpConnector::new(tor_client.clone(), tls_connector);
    let http = hyper::Client::builder().build::<_, Body>(tor_connector);

    let url = "https://httpstat.us/random/200,500-504";
    println!("{} requesting {} via Tor...", socks_port, url);
    let mut resp = http.get(url.try_into()?).await?;

    println!("{} status: {}", socks_port, resp.status());
    ready.store(true, SeqCst);

    let proxy = tokio::spawn(run_socks_proxy(runtime, tor_client, socks_port));

    Ok(())
}
