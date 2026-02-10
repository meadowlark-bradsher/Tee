use std::sync::Arc;

use tonic::transport::Server;
use tracing_subscriber::EnvFilter;

use tee::config::Config;
use tee::proto::tee_server::TeeServer;
use tee::service::TeeService;
use tee::store::memory::InMemoryStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::default();
    let store = Arc::new(InMemoryStore::new());

    tracing::info!("Tee server listening on {}", config.listen_addr);

    Server::builder()
        .add_service(TeeServer::new(TeeService::new(store)))
        .serve(config.listen_addr)
        .await?;

    Ok(())
}
