use log::error;
use std::env;

mod card;
mod card_client;
mod message;
use card_client::CardClient;

fn print_usage() {
    println!(
        "Usage: cargo run --package card -- <company_id> <card_id> --province <province_id> --station <station_id>"
    );
    println!();
    println!("Examples:");
    println!("  cargo run --package card -- 1 100 --province 0 --station 5");
    println!("  cargo run --package card -- 2 200 --province 3 --station 10");
    println!();
    println!("Available provinces (0-15):");
    println!("  0: Buenos Aires     8: Mendoza");
    println!("  1: Catamarca        9: Misiones");
    println!("  2: Chaco           10: Neuquén");
    println!("  3: Córdoba         11: Río Negro");
    println!("  4: Entre Ríos      12: Salta");
    println!("  5: Formosa         13: Santa Cruz");
    println!("  6: Jujuy           14: Santiago del Estero");
    println!("  7: La Pampa        15: Tierra del Fuego");
    println!();
    println!("Stations: 0-99 per province");
}

fn parse_args() -> Result<(u32, u32, u32, u32), String> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 7 {
        return Err("Insufficient arguments".to_string());
    }

    // Parse company_id and card_id
    let company_id: u32 = args[1]
        .parse()
        .map_err(|_| "Company ID must be a valid number")?;

    let card_id: u32 = args[2]
        .parse()
        .map_err(|_| "Card ID must be a valid number")?;

    let mut province_id: Option<u32> = None;
    let mut station_id: Option<u32> = None;

    // Parse flags
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--province" => {
                if i + 1 >= args.len() {
                    return Err("--province requires a value".to_string());
                }
                let pid: u32 = args[i + 1]
                    .parse()
                    .map_err(|_| "Province ID must be a valid number")?;
                if pid > 15 {
                    return Err("Province ID must be between 0-15".to_string());
                }
                province_id = Some(pid);
                i += 2;
            }
            "--station" => {
                if i + 1 >= args.len() {
                    return Err("--station requires a value".to_string());
                }
                let sid: u32 = args[i + 1]
                    .parse()
                    .map_err(|_| "Station ID must be a valid number")?;
                if sid > 99 {
                    return Err("Station ID must be between 0-99".to_string());
                }
                station_id = Some(sid);
                i += 2;
            }
            _ => {
                return Err(format!("Unknown argument: {}", args[i]));
            }
        }
    }

    let province_id = province_id.ok_or("--province argument is required")?;
    let station_id = station_id.ok_or("--station argument is required")?;

    Ok((company_id, card_id, province_id, station_id))
}

#[actix_rt::main]
async fn main() {
    env_logger::init();

    let (company_id, card_id, province_id, station_id) = match parse_args() {
        Ok(values) => values,
        Err(e) => {
            error!("Error: {}", e);
            print_usage();
            return;
        }
    };

    println!(
        "🏧 Starting Card {} for Company {} targeting Province {} Station {}",
        card_id, company_id, province_id, station_id
    );

    let card_client = CardClient::new(company_id, card_id, province_id, station_id);
    match card_client.run().await {
        Ok(()) => (),
        Err(e) => error!("ERROR: {}", e),
    }
}
