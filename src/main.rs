use std::{
    net::SocketAddr,
    println,
    sync::{atomic::AtomicU32, Arc},
    time::Duration,
};

use arti::socks::run_socks_proxy;
use arti_client::{TorClient, TorClientConfig};
use futures::future::join_all;
use tokio::{
    net::{TcpListener, TcpStream},
    time::sleep,
};
use tor_rtcompat::{BlockOn, Runtime};

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

    let selected_proxy: Arc<AtomicU32> = Default::default();

    tokio::spawn(run_proxy(
        runtime.clone(),
        tor_client.isolated_client(),
        9051,
    ));

    tokio::spawn(run_proxy(
        runtime.clone(),
        tor_client.isolated_client(),
        9052,
    ));

    let listener = TcpListener::bind("127.0.0.1:9050").await.unwrap();
    while let Ok((mut ingress, _)) = listener.accept().await {
        tokio::spawn(async move {
            println!("New connection");
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
            println!("Done");
        });
    }
}

/// Run main proxy loop
async fn run_proxy<R: Runtime>(runtime: R, tor_client: TorClient<R>, socks_port: u16) {
    match run_socks_proxy(runtime, tor_client, socks_port).await {
        Ok(()) => println!("Proxy exited from port {}", socks_port),
        Err(error) => println!("Failed to start proxy on port {}, {}", socks_port, error),
    };
}
