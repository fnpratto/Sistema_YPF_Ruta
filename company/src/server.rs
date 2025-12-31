use std::path::Path;

use crate::persistence::PersistenceActor;
use crate::regional::RegionalAdminHandler;
use crate::registry::{CreateOrConnectCompany, RegistryActor};
use actix::prelude::*;
use common::constants::PROVINCES;
use common::{company_debug, company_error, company_info};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[allow(dead_code)]
pub struct Server {
    address: String,
    registry: Addr<RegistryActor>,
    persistence: Addr<PersistenceActor>,
}

impl Server {
    pub fn new(addr: &str) -> Self {
        let persistence = PersistenceActor.start();
        let registry = RegistryActor::new(persistence.clone()).start();

        Server {
            address: addr.to_string(),
            registry,
            persistence,
        }
    }

    pub async fn restore_companies(&self) {
        let registry = self.registry.clone();

        let data_dir = Path::new("data");
        if !data_dir.exists() {
            company_debug!("[SERVER] No persisted companies found");
            return;
        }

        let mut entries = match fs::read_dir(data_dir).await {
            Ok(e) => e,
            Err(_) => return,
        };

        company_info!("[SERVER] Restoring companies from disk...");

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            #[allow(clippy::collapsible_if)]
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if let Ok(company_id) = stem.parse::<u32>() {
                    registry.do_send(CreateOrConnectCompany {
                        company_id,
                        admin_stream: None,
                    });
                }
            }
        }

        company_debug!("[SERVER] Companies restoring completed!");
    }

    pub async fn connect_to_regional_admins(&self) {
        company_info!("[SERVER] Connecting to RegionalAdmins...");

        for province in PROVINCES.iter() {
            let addr = province.address;
            let province_id = province.id;

            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    company_info!(
                        "[SERVER] Connected to RegionalAdmin {} ({})",
                        province_id,
                        province.name
                    );

                    let handshake = format!("COMPANY {}\n", self.address.clone());
                    if let Err(e) = stream.write_all(handshake.as_bytes()).await {
                        company_error!(
                            "[SERVER] Failed sending handshake to RegionalAdmin {}: {}",
                            province_id,
                            e
                        );
                        continue;
                    }

                    let registry = self.registry.clone();
                    let persistence = self.persistence.clone();

                    actix_rt::spawn(async move {
                        RegionalAdminHandler::create(|ctx| {
                            RegionalAdminHandler::new(
                                stream,
                                ctx,
                                province_id.to_string(),
                                registry,
                                persistence,
                            )
                        });
                    });
                }

                Err(_) => {
                    company_debug!(
                        "[SERVER] RegionalAdmin {} ({}) not available!",
                        province_id,
                        province.name
                    );
                }
            }
        }

        company_debug!("[SERVER] Finished RegionalAdmin bootstrap connections");
    }

    pub async fn start_listener(self) {
        company_info!("[SERVER] Listening initializated in: {}", self.address);
        company_info!("[SERVER] Press Ctrl+C to stop.");

        let listener = TcpListener::bind(&self.address)
            .await
            .expect("[SERVER] ERROR at binding");

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    company_debug!("[SERVER] New connection of: {}", addr);

                    let registry = self.registry.clone();
                    let persistence = self.persistence.clone();

                    actix_rt::spawn(async move {
                        // leo la primera línea para saber qué tipo de conexion es
                        let mut reader = tokio::io::BufReader::new(stream);
                        let mut first_line = String::new();

                        if reader.read_line(&mut first_line).await.is_err() {
                            company_error!("Error reading type of client");
                            return;
                        }

                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        //TODO
                        if parts.len() < 2 {
                            //company_error!("Invalid Connection, missing ID");
                            return;
                        }
                        let connect_type = parts[0];
                        let id = parts[1].to_string();

                        let stream = reader.into_inner();

                        match connect_type {
                            "COMPANY" => {
                                let company_id = id.parse::<u32>().unwrap_or(0);
                                registry.do_send(CreateOrConnectCompany {
                                    company_id,
                                    admin_stream: Some(stream),
                                });
                            }

                            "REGIONAL" => {
                                RegionalAdminHandler::create(|ctx| {
                                    RegionalAdminHandler::new(
                                        stream,
                                        ctx,
                                        id,
                                        registry.clone(),
                                        persistence.clone(),
                                    )
                                });
                            }

                            _ => {
                                company_error!("Unknown type: {}", connect_type);
                            }
                        }
                    });
                }

                Err(e) => {
                    company_error!("[SERVER] Error accepting connection: {}", e);
                }
            }
        }
    }
}
