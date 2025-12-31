use log::{debug, error};
use std::env;

mod admin_actor;
mod admin_client;
use admin_client::CompanyAdminClient;

#[actix_rt::main]
async fn main() {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        debug!("Uso: company_admin <company_id>");
        return;
    }

    let company_id: u32 = match args[1].parse() {
        Ok(id) => id,
        Err(_) => {
            debug!("Error: Company ID must be a valid number");
            return;
        }
    };

    let client = CompanyAdminClient::new(company_id);
    match client.run().await {
        Ok(()) => (),
        Err(e) => error!("ERROR: {}", e),
    }
}
