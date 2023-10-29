use std::{
    println,
    sync::{
        atomic::{AtomicBool, Ordering::SeqCst},
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
    task::JoinHandle,
    time::sleep,
};
use tor_rtcompat::{BlockOn, Runtime};

// https://gitlab.torproject.org/tpo/core/arti/-/issues/715
#[cfg(not(target_vendor = "apple"))]
use tls_api_native_tls::TlsConnector;
#[cfg(target_vendor = "apple")]
use tls_api_openssl::TlsConnector;

#[derive(Debug, Clone)]
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

    for i in 0..20 {
        let ready: Arc<AtomicBool> = Default::default();
        let port = 9051 + i;
        ready_proxies.push(Proxy {
            ready: ready.clone(),
            port: port,
        });
        tokio::spawn(run_proxy(runtime.clone(), tor_client.clone(), ready, port));
    }

    let mut connections: Vec<JoinHandle<()>> = Default::default();

    let listener = TcpListener::bind("127.0.0.1:9050").await.unwrap();
    while let Ok((mut ingress, _)) = listener.accept().await {
        let ready_proxies = ready_proxies.clone();
        connections.push(tokio::spawn(async move {
            let proxy = ready_proxies
                .into_iter()
                .find(|proxy| proxy.ready.load(SeqCst));

            let address = format!("127.0.0.1:{}", proxy.unwrap().port);
            println!("Proxying connection through {}", address);

            let mut egress = TcpStream::connect(address).await.unwrap();
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
            println!("Done");
        }));
    }

    join_all(connections.into_iter()).await;
}

async fn check_valid<R: Runtime>(
    http: hyper::Client<ArtiHttpConnector<R, TlsConnector>>,
) -> Result<bool> {
    // let url = "https://lite.duckduckgo.com/lite/?q=test";
    let url = "https://httpstat.us/random/200,500";
    let response = http.get(url.try_into()?).await?;
    let status = response.status();

    Ok(status == 200)
}

/// Run main proxy loop
async fn run_proxy<R: Runtime>(
    runtime: R,
    mut tor_client: TorClient<R>,
    ready: Arc<AtomicBool>,
    socks_port: u16,
) -> Result<()> {
    let mut proxy = tokio::spawn(run_socks_proxy(
        runtime.clone(),
        tor_client.clone(),
        socks_port,
    ));

    loop {
        if ready.load(SeqCst) {
            let tls_connector = TlsConnector::builder()?.build()?;
            let tor_connector = ArtiHttpConnector::new(tor_client.clone(), tls_connector);
            let http = hyper::Client::builder().build::<_, Body>(tor_connector);

            // println!("{} checking", socks_port);

            if !check_valid(http).await? {
                proxy.abort();
                ready.store(false, SeqCst);
                println!("{} is bad", socks_port);
            } else {
                sleep(Duration::from_secs(60 * 2)).await;
            }
        }

        if !ready.load(SeqCst) {
            println!("Refreshing circuit");
            tor_client = tor_client.isolated_client();

            let tls_connector = TlsConnector::builder()?.build()?;
            let tor_connector = ArtiHttpConnector::new(tor_client.clone(), tls_connector);
            let http = hyper::Client::builder().build::<_, Body>(tor_connector);

            if check_valid(http).await? {
                proxy = tokio::spawn(run_socks_proxy(
                    runtime.clone(),
                    tor_client.clone(),
                    socks_port,
                ));
                ready.store(true, SeqCst);
                println!("{} is good", socks_port);
                sleep(Duration::from_secs(60 * 2)).await;
            } else {
                proxy.abort();
                println!("{} is bad", socks_port);
            }
        }
    }
}
