use crate::message::UpdatePumps;
use crate::pump::Pump;
use common::constants::{PROVINCES, PUMP_BASE_PORT, get_province_name, get_regional_admin_address};
use common::message::Start;
use log::{error, info};
use std::{collections::HashMap, env, process};

mod message;
pub mod pump;

#[derive(Debug)]
struct LogConfig {
    amount: u32,
    regional_admin_only: bool,
    province_id: Option<u32>,
    service_station_id: u32,
}

fn parse_args() -> LogConfig {
    let args: Vec<String> = env::args().collect();

    if args.len() < 6 {
        error!(
            "Usage: {} <amount> --province <province_id> --station <station_id> [--regional-admin-only]",
            args[0]
        );
        error!("Available provinces (0-15):");
        for i in 0..16 {
            if let Some(name) = get_province_name(i) {
                error!("  {}: {}", i, name);
            }
        }
        error!("Station ID should be a unique identifier for this gas station");
        process::exit(1);
    }

    let amount = match args[1].parse::<u32>() {
        Ok(id) => id,
        Err(_) => {
            error!("Error: <amount> must be a positive integer.");
            process::exit(1);
        }
    };

    let mut regional_admin_only = false;
    let mut province_id = None;
    let mut service_station_id = None;
    let mut i = 2;

    while i < args.len() {
        match args[i].as_str() {
            "--regional-admin-only" => {
                regional_admin_only = true;
                i += 1;
            }
            "--province" => {
                if i + 1 >= args.len() {
                    error!("Error: --province requires a province ID");
                    process::exit(1);
                }
                match args[i + 1].parse::<u32>() {
                    Ok(pid) => {
                        if pid > 15 {
                            error!("Error: Province ID must be between 0-15");
                            process::exit(1);
                        }
                        province_id = Some(pid);
                        i += 2;
                    }
                    Err(_) => {
                        error!("Error: Province ID must be a valid number (0-15)");
                        process::exit(1);
                    }
                }
            }
            "--station" => {
                if i + 1 >= args.len() {
                    error!("Error: --station requires a station ID");
                    process::exit(1);
                }
                match args[i + 1].parse::<u32>() {
                    Ok(sid) => {
                        service_station_id = Some(sid);
                        i += 2;
                    }
                    Err(_) => {
                        error!("Error: Station ID must be a valid number");
                        process::exit(1);
                    }
                }
            }
            _ => {
                error!("Error: Unknown argument '{}'", args[i]);
                process::exit(1);
            }
        }
    }

    if province_id.is_none() {
        error!("Error: --province parameter is required");
        process::exit(1);
    }

    if service_station_id.is_none() {
        error!("Error: --station parameter is required");
        process::exit(1);
    }

    LogConfig {
        amount,
        regional_admin_only,
        province_id,
        service_station_id: service_station_id.unwrap(),
    }
}

fn init_logger(regional_admin_only: bool) {
    use env_logger::Env;
    use log::LevelFilter;

    if regional_admin_only {
        env_logger::Builder::from_env(Env::default())
            .filter(None, LevelFilter::Off)
            .filter_module("gas_pump", LevelFilter::Off)
            .filter_level(LevelFilter::Info)
            .format(|buf, record| {
                use std::io::Write;
                let target = record.target();
                let message = record.args().to_string();

                if target.contains("regional")
                    || message.to_lowercase().contains("regional")
                    || message.to_lowercase().contains("connection")
                    || message.to_lowercase().contains("payment")
                    || message.to_lowercase().contains("expense")
                        && message.to_lowercase().contains("response")
                {
                    writeln!(buf, "[REGIONAL_ADMIN] {} - {}", record.level(), message)
                } else {
                    Ok(())
                }
            })
            .init();

        info!("Regional Admin only logging enabled");
    } else {
        env_logger::init();
    }
}

fn calculate_pump_port(province_id: u32, service_station_id: u32, pump_index: u32) -> u32 {
    PUMP_BASE_PORT + (province_id * 1000) + (service_station_id * 10) + pump_index
}

#[actix_rt::main]
async fn main() {
    let config = parse_args();
    init_logger(config.regional_admin_only);

    let province_id = config.province_id.unwrap();
    let service_station_id = config.service_station_id;
    let province_name = get_province_name(province_id).unwrap_or("Unknown");

    info!(
        "Starting {} pumps for Service Station {} in province {} ({})",
        config.amount, service_station_id, province_name, province_id
    );

    let mut addr_pumps: HashMap<u32, actix::Addr<Pump>> = HashMap::new();
    let mut pump_ports: HashMap<u32, bool> = HashMap::new();

    let mut regional_admin_addrs = Vec::new();
    let total_provinces = PROVINCES.len();

    for i in 0..total_provinces {
        let current_province_id = (province_id as usize + i) % total_provinces;
        if let Some(addr) = get_regional_admin_address(current_province_id as u32) {
            regional_admin_addrs.push(addr.to_string());
        }
    }

    for pump_index in 0..config.amount {
        let pump_port = calculate_pump_port(province_id, service_station_id, pump_index);
        let pump_address = format!("127.0.0.1:{}", pump_port);

        info!(
            "Creating pump {} on port {} (Province: {}, Station: {}, Index: {})",
            pump_index, pump_port, province_id, service_station_id, pump_index
        );

        let pump_addr = Pump::spawn_with_setup(
            false,
            None,
            pump_address.clone(),
            regional_admin_addrs.clone(),
            service_station_id,
        )
        .await;

        addr_pumps.insert(pump_port, pump_addr);
        pump_ports.insert(pump_port, false);
    }

    for addr in addr_pumps.values() {
        addr.do_send(UpdatePumps {
            addr_pumps: addr_pumps.clone(),
        });
    }

    for addr in addr_pumps.values() {
        addr.do_send(Start);
    }

    info!(
        "All {} pumps started for Service Station {}",
        config.amount, service_station_id
    );

    tokio::signal::ctrl_c().await.unwrap();
    info!("Shutting down Service Station {}", service_station_id);
}
