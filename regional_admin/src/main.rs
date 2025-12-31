use std::env;
pub mod connection_actor;
pub mod message;
pub mod regional_admin;
use actix::Actor;

use common::constants::{get_province_name, get_regional_admin_address, is_valid_province_id};
use common::{crash_info, election_info, leader_info};

use crate::regional_admin::RegionalAdmin;

#[actix_rt::main]
async fn main() {
    env_logger::init();

    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        crash_info!("Usage: regional_admin <province_id>");
        crash_info!("Available provinces (0-15):");
        for i in 0..16 {
            if let Some(name) = get_province_name(i) {
                crash_info!("  {}: {}", i, name);
            }
        }
        return;
    }

    let province_id: u32 = match args[1].parse() {
        Ok(id) => id,
        Err(_) => {
            crash_info!("Error: Province ID must be a valid number (0-15)");
            return;
        }
    };

    if !is_valid_province_id(province_id) {
        crash_info!(
            "Error: Invalid province ID {}. Must be between 0-15",
            province_id
        );
        crash_info!("Available provinces:");
        for i in 0..16 {
            if let Some(name) = get_province_name(i) {
                crash_info!("  {}: {}", i, name);
            }
        }
        return;
    }

    let listen_addr = match get_regional_admin_address(province_id) {
        Some(addr) => addr.to_string(),
        None => {
            crash_info!(
                "Error: Could not get address for province ID {}",
                province_id
            );
            return;
        }
    };

    let province_name = get_province_name(province_id).unwrap_or("Unknown");

    election_info!(
        "Starting RegionalAdmin for {} (ID: {}) on {}",
        province_name,
        province_id,
        listen_addr
    );

    let reg_admin = RegionalAdmin::new(province_id, listen_addr.clone());
    let _ = reg_admin.start();

    leader_info!(
        "RegionalAdmin for {} system running on {}. Press Ctrl+C to stop.",
        province_name,
        listen_addr
    );

    tokio::signal::ctrl_c().await.unwrap();
    crash_info!("Shutting down RegionalAdmin for {} system.", province_name);
}
