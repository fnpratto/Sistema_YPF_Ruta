#[allow(unused_imports)]
use crate::*;
#[allow(unused_imports)]
use common::constants::{COMPANY_LEADER_PORT, LOCALHOST, PUMP_BASE_PORT, REGIONAL_ADMIN_PORT};
#[allow(unused_imports)]
use common::{performance_debug, performance_error, performance_info};
use std::io;
#[allow(unused_imports)]
use std::io::Write;
use std::net::TcpStream;
use std::process::Child;
#[allow(unused_imports)]
use std::process::Stdio;
use std::time::Duration;

#[allow(dead_code)]
const CANT_REGIONAL_ADMINS: u32 = 16; // Reducir de 16 a 2
#[allow(dead_code)]
const CANT_STATIONS: u32 = 20; // Reducir de 70 a 4
#[allow(dead_code)]
const CANT_COMPANYS: u32 = 10; // Reducir de 10 a 2
#[allow(dead_code)]
const CANT_CARDS: u32 = 20; // Aumentar de 4 a 20
#[allow(dead_code)]
const CANT_CARDS_STRESS: u32 = 20;

#[allow(dead_code)]
const SHORT_SLEEP: Duration = Duration::from_secs(1);
#[allow(dead_code)]
const MEDIUM_SLEEP: Duration = Duration::from_secs(3);
#[allow(dead_code)]
const LONG_SLEEP: Duration = Duration::from_secs(10);

pub fn setup_card(i: u32) -> (u32, u32, u32, u32) {
    let stations_per_province = CANT_STATIONS / CANT_REGIONAL_ADMINS; // 64/16 = 4
    let _cards_per_company = CANT_CARDS / CANT_COMPANYS; // 40/10 = 4

    let province_id = i % CANT_REGIONAL_ADMINS; // 0-15
    let service_station_id = (i / CANT_REGIONAL_ADMINS) % stations_per_province; // 0
    let company_id = i % CANT_COMPANYS; // 0-9
    let card_id = (i / CANT_COMPANYS) + 1; // 1-4

    (province_id, service_station_id, company_id, card_id)
}

pub fn setup_card_with_balance(i: u32) -> (u32, u32, u32, u32) {
    setup_card(i)
}

pub fn wait_for_server_ready(port: u32) -> Result<(), std::io::Error> {
    let addr = format!("{}:{}", LOCALHOST, port);
    performance_info!("Verificando conexión en {}...", addr);

    for _ in 0..50 {
        match TcpStream::connect(&addr) {
            Ok(_) => {
                return Ok(());
            }
            Err(_e) => {
                std::thread::sleep(SHORT_SLEEP);
                continue;
            }
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Error de tiempo de espera al conectar.",
    ))
}

pub fn kill_child_process(mut process: Child, name: &str) -> Result<(), io::Error> {
    performance_info!("Cerrando proceso {}...", name);
    process.kill()
}

pub fn close_ypf_system(
    server_process: Child,
    company_admins: Option<Vec<Child>>,
    regional_admins: Option<Vec<Child>>,
    pumps: Option<Vec<Child>>,
    cards: Option<Vec<Child>>,
) {
    performance_info!("Cerrando el Sistema YPF...");

    if let Some(cards) = cards {
        for card in cards {
            match kill_child_process(card, "Tarjeta") {
                Ok(()) => performance_info!("✅ Tarjeta cerrada."),
                Err(kill_err) => {
                    performance_error!("❌ Error al cerrar una Tarjeta: {}", kill_err)
                }
            }
        }
    }

    if let Some(company_admins) = company_admins {
        for admin in company_admins {
            match kill_child_process(admin, "Admin Empresa") {
                Ok(()) => performance_info!("✅ Admin Empresa cerrado."),
                Err(kill_err) => {
                    performance_error!("❌ Error al cerrar un Admin Empresa: {}", kill_err)
                }
            }
        }
    }

    if let Some(pumps) = pumps {
        for pump in pumps {
            match kill_child_process(pump, "Surtidor") {
                Ok(()) => performance_info!("✅ Surtidor cerrado."),
                Err(kill_err) => {
                    performance_error!("❌ Error al cerrar una Estación: {}", kill_err)
                }
            }
        }
    }

    if let Some(regional_admins) = regional_admins {
        for admin in regional_admins {
            match kill_child_process(admin, "Admin Regional") {
                Ok(()) => performance_info!("✅ Admin Regional cerrado."),
                Err(kill_err) => {
                    performance_error!("❌ Error al cerrar un Admin Regional: {}", kill_err)
                }
            }
        }
    }

    match kill_child_process(server_process, "Servidor") {
        Ok(()) => performance_info!("✅ Servidor cerrado."),
        Err(kill_err) => performance_error!("❌ Error al cerrar el Servidor: {}", kill_err),
    }

    performance_info!("Sistema YPF cerrado.");
}

#[test]
fn test_system_load() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    performance_info!("Iniciando servidor");

    let server_process = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("company")
        .spawn()
        .expect("❌ Error al iniciar el Servidor.");

    match wait_for_server_ready(COMPANY_LEADER_PORT as u32) {
        Ok(()) => {
            performance_info!("✅ Servidor levantado.");
        }
        Err(e) => {
            // Sleep para probar casos de error.
            std::thread::sleep(LONG_SLEEP);

            close_ypf_system(server_process, None, None, None, None);
            performance_error!("❌ Fallo de sincronización con el Servidor: {}", e);
            panic!("❌ No se pudo iniciar el servidor.");
        }
    };

    std::thread::sleep(MEDIUM_SLEEP);
    performance_info!("✅ Servidor levantado.");

    performance_info!("Iniciando Administradores de Empresa...");

    let mut company_admins: Vec<std::process::Child> = Vec::with_capacity(CANT_COMPANYS as usize);

    for id in 0..CANT_COMPANYS {
        performance_info!("Iniciando Admin Empresa {}...", id);

        let mut company_admin_process = std::process::Command::new("cargo")
            .arg("run")
            .arg("--package")
            .arg("company_admin")
            .arg("--")
            .arg(id.to_string())
            .spawn()
            .expect("❌ Error al iniciar un Admin Empresa.");

        std::thread::sleep(SHORT_SLEEP);

        match company_admin_process.try_wait() {
            Ok(None) => {
                performance_info!("✅ Admin Empresa {} levantado y conectado.", id);
                company_admins.push(company_admin_process);
            }
            _ => {
                performance_error!("❌ Admin Empresa {} falló.", id);

                close_ypf_system(server_process, Some(company_admins), None, None, None);

                panic!("❌ Error al levantar los Administradores de Empresa.");
            }
        }
    }

    performance_info!("Iniciando Administradores Regionales...");

    let mut regional_admins: Vec<std::process::Child> =
        Vec::with_capacity(CANT_REGIONAL_ADMINS as usize);

    for id in 0..CANT_REGIONAL_ADMINS {
        let current_port = REGIONAL_ADMIN_PORT + id + 1;
        let id_str = id.to_string();

        performance_info!(
            "Iniciando Admin Regional {} en puerto {}.",
            id,
            current_port
        );

        let regional_admin_process = std::process::Command::new("cargo")
            .arg("run")
            .arg("--package")
            .arg("regional_admin")
            .arg("--")
            .arg(id_str)
            .spawn()
            .expect("❌ Error al iniciar el Admin Regional.");

        match wait_for_server_ready(current_port) {
            Ok(()) => {
                performance_info!("✅ Admin Regional {} levantado y conectado.", id);
                regional_admins.push(regional_admin_process);
            }
            Err(e) => {
                performance_error!(
                    "❌ Fallo de sincronización del Admin Regional {}: {}",
                    id,
                    e
                );

                // Sleep para probar casos de error.
                std::thread::sleep(LONG_SLEEP);

                close_ypf_system(
                    server_process,
                    Some(company_admins),
                    Some(regional_admins),
                    None,
                    None,
                );

                panic!(
                    "❌ Fallo en el setup. No se pudo iniciar el Admin Regional {}.",
                    id
                );
            }
        };
    }

    std::thread::sleep(MEDIUM_SLEEP);
    performance_info!(
        "✅ Los {} Administradores Regionales están levantados.",
        CANT_REGIONAL_ADMINS
    );

    performance_info!("Iniciando Estaciones...");

    let mut pumps: Vec<std::process::Child> = Vec::with_capacity(CANT_STATIONS as usize);

    for province_id in 0..CANT_REGIONAL_ADMINS {
        for service_station_id in 0..(CANT_STATIONS / CANT_REGIONAL_ADMINS) {
            let pump_index = 0;
            let amount_of_pumps = 6;
            let port =
                PUMP_BASE_PORT + (province_id * 1000) + (service_station_id * 10) + pump_index;

            match TcpStream::connect(format!("{}:{}", LOCALHOST, port)) {
                Ok(_) => {
                    performance_error!("❌ Puerto {} ya está en uso, saltando...", port);
                    continue;
                }
                Err(_) => {
                    performance_debug!("✅ Puerto {} libre, creando pump...", port);
                }
            }

            performance_info!(
                "Iniciando {} surtidores en Estación {} en provincia {} en puerto base {}...",
                amount_of_pumps,
                service_station_id,
                province_id,
                port
            );

            let pump_process = std::process::Command::new("cargo")
                .arg("run")
                .arg("--package")
                .arg("gas_pump")
                .arg("--")
                .arg(amount_of_pumps.to_string())
                .arg("--province")
                .arg(province_id.to_string())
                .arg("--station")
                .arg(service_station_id.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("❌ Error al iniciar un Surtidor.");

            std::thread::sleep(Duration::from_secs(3));

            match wait_for_server_ready(port) {
                Ok(()) => {
                    performance_info!(
                        "✅ Estación {} en provincia {} levantada en puerto {}.",
                        service_station_id,
                        province_id,
                        port
                    );
                    pumps.push(pump_process);
                }
                Err(e) => {
                    performance_error!(
                        "❌ Fallo de sincronización con la Estación {} en provincia {}: {}",
                        service_station_id,
                        province_id,
                        e
                    );

                    continue;
                }
            };
        }
    }

    std::thread::sleep(MEDIUM_SLEEP);
    performance_info!("✅ Los Surtidores están levantados.");

    performance_info!("Iniciando Tarjeta");

    let mut cards: Vec<std::process::Child> = Vec::with_capacity(CANT_CARDS as usize);

    for i in 0..CANT_CARDS {
        performance_info!("Iniciando Tarjeta {}...", i);

        let (province_id, service_station_id, company_id, card_id) = setup_card_with_balance(i);

        let mut card = std::process::Command::new("cargo")
            .arg("run")
            .arg("--package")
            .arg("card")
            .arg("--")
            .arg(company_id.to_string())
            .arg(card_id.to_string())
            .arg("--province")
            .arg(province_id.to_string())
            .arg("--station")
            .arg(service_station_id.to_string())
            .stdin(Stdio::piped())
            .spawn()
            .expect("❌ Error al iniciar una Tarjeta.");

        std::thread::sleep(SHORT_SLEEP);

        match card.try_wait() {
            Ok(None) => {
                performance_info!("✅ Tarjeta {} levantada", i);
                cards.push(card);
            }
            _ => {
                performance_error!("❌ Tarjeta número {} falló.", i);

                close_ypf_system(
                    server_process,
                    Some(company_admins),
                    Some(regional_admins),
                    Some(pumps),
                    Some(cards),
                );

                panic!("❌ Error al levantar la Tarjeta.");
            }
        }
    }

    std::thread::sleep(MEDIUM_SLEEP);
    performance_info!("✅ Tarjetas levantadas.");
    performance_info!("Simulando carga en el sistema...");

    for (index, card) in cards.iter_mut().enumerate() {
        performance_info!("Simulando uso de Tarjeta {}...", index);

        if let Some(mut stdin) = card.stdin.take() {
            let safe_liters = 1;
            let mock_input = format!("no\n{}\n", safe_liters);

            let mut retries = 3;
            let mut success = false;

            while retries > 0 && !success {
                performance_info!(
                    "Enviando transacción de {} litro a tarjeta {} (Intento {})",
                    safe_liters,
                    index,
                    4 - retries
                );

                match stdin.write_all(mock_input.as_bytes()) {
                    Ok(_) => {
                        performance_info!("✅ Input enviado a tarjeta {}", index);
                        success = true;
                    }
                    Err(e) => {
                        performance_error!(
                            "❌ Error enviando input a tarjeta {}: {}. Reintentando...",
                            index,
                            e
                        );
                        retries -= 1;
                        std::thread::sleep(Duration::from_secs(2));
                    }
                }
            }

            if !success {
                performance_error!(
                    "❌ Transacción fallida después de 3 intentos para tarjeta {}.",
                    index
                );
            }

            drop(stdin);

            std::thread::sleep(Duration::from_secs(5));
        } else {
            performance_error!("❌ No se pudo obtener stdin de tarjeta {}", index);
        }
    }

    // Aumentar tiempo de espera para que las transacciones se completen
    std::thread::sleep(Duration::from_secs(10));
    performance_info!("Cerrando el Sistema YPF...");

    close_ypf_system(
        server_process,
        Some(company_admins),
        Some(regional_admins),
        Some(pumps),
        Some(cards),
    );

    performance_info!("Prueba de performance finalizada exitosamente!");
}

#[test]
fn test_system_minimal() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    performance_info!("🧪 Test mínimal: 1 servidor + 1 admin + 1 pump + 1 tarjeta");

    let server_process = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("company")
        .spawn()
        .expect("Error servidor");

    std::thread::sleep(Duration::from_secs(5));

    let company_admin = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("company_admin")
        .arg("--")
        .arg("0")
        .spawn()
        .expect("Error company admin");

    std::thread::sleep(Duration::from_secs(3));

    let regional_admin = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("regional_admin")
        .arg("--")
        .arg("0")
        .spawn()
        .expect("Error regional admin");

    std::thread::sleep(Duration::from_secs(3));

    let pump = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("gas_pump")
        .arg("--")
        .arg("1")
        .arg("--province")
        .arg("0")
        .arg("--station")
        .arg("0")
        .spawn()
        .expect("Error pump");

    std::thread::sleep(Duration::from_secs(5));

    let mut card = std::process::Command::new("cargo")
        .arg("run")
        .arg("--package")
        .arg("card")
        .arg("--")
        .arg("0")
        .arg("1")
        .arg("--province")
        .arg("0")
        .arg("--station")
        .arg("0")
        .stdin(Stdio::piped())
        .spawn()
        .expect("Error card");

    std::thread::sleep(Duration::from_secs(3));

    if let Some(mut stdin) = card.stdin.take() {
        performance_info!("Enviando transacción de 1 litro");
        let _ = stdin.write_all(b"no\n1\n");
        drop(stdin);
    }

    std::thread::sleep(Duration::from_secs(10));

    close_ypf_system(
        server_process,
        Some(vec![company_admin]),
        Some(vec![regional_admin]),
        Some(vec![pump]),
        Some(vec![card]),
    );

    performance_info!("✅ Test mínimal completado");
}
