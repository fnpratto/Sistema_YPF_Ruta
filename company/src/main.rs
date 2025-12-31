use common::company_info;
use common::constants::{COMPANY_LEADER_PORT, LOCALHOST};
use company::server::Server;

#[actix_rt::main]
async fn main() {
    env_logger::init();
    company_info!("[SERVER] Starting system...");
    let server = Server::new(format!("{}:{}", LOCALHOST, COMPANY_LEADER_PORT).as_str());

    server.restore_companies().await;
    server.connect_to_regional_admins().await;

    actix_rt::spawn(async move {
        server.start_listener().await;
    });

    tokio::signal::ctrl_c().await.unwrap();
    company_info!("\n[SERVER] Closing server!");
}
