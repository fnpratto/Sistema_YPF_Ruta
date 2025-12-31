use actix::{Actor, Addr};
use log::debug;
use std::io::{Error, ErrorKind, Result, Write, stdin, stdout};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::card::Card;
use crate::message::{FuelComplete, FuelRequest};
use common::message::{GasPumpYPFMessage, GetPendingCardResult, MsgType};
use common::tcp_protocol::TCPProtocol;

pub struct CardClient {
    company_id: u32,
    card_id: u32,
    province_id: u32,
    station_id: u32,
}

impl CardClient {
    pub fn new(company_id: u32, card_id: u32, province_id: u32, station_id: u32) -> Self {
        Self {
            company_id,
            card_id,
            province_id,
            station_id,
        }
    }

    pub async fn run(&self) -> Result<()> {
        debug!("Checking for pending results...");
        if let Some((pump_addr, pending_msg)) = self.check_pending_result_broadcast().await? {
            println!(
                "✅ Pending result received from {}: {:?}",
                pump_addr, pending_msg
            );
            return Ok(());
        }

        debug!("Looking for available gas pump...");

        let (start_port, end_port) = (self.get_start_port(), self.get_end_port());

        println!(
            "🔍 Searching for pumps in station {} of province {} (ports {}-{})",
            self.station_id, self.province_id, start_port, end_port
        );

        let gas_pump_address = self
            .find_available_gas_pump("127.0.0.1", start_port, end_port)
            .await?;
        println!("⛽ Gas pump chosen: {}", gas_pump_address);

        // Create card actor and process fuel request
        let (tx, mut rx) = mpsc::unbounded_channel::<FuelComplete>();
        let card = Card::new(self.company_id, self.card_id, &gas_pump_address, tx)?;
        let card_addr = card.start();

        // Process new fuel request
        let liters: u32 = self.choose_liters_to_load()?;
        self.send_fuel_request(&card_addr, liters);
        self.waiting_for_charge_completion(&mut rx).await;

        drop(card_addr);

        Ok(())
    }

    fn get_start_port(&self) -> u16 {
        (10000 + self.province_id * 1000 + self.station_id * 10) as u16
    }

    fn get_end_port(&self) -> u16 {
        (10000 + self.province_id * 1000 + self.station_id * 10 + 9) as u16
    }

    async fn find_available_gas_pump(
        &self,
        base_addr: &str,
        start_port: u16,
        end_port: u16,
    ) -> Result<String> {
        for port in start_port..=end_port {
            let addr = format!("{}:{}", base_addr, port);
            debug!("Trying to connect to {}...", addr);

            match tokio::time::timeout(
                Duration::from_millis(200),
                tokio::net::TcpStream::connect(&addr),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    debug!("✅ Gas pump found at {}", addr);
                    drop(stream);
                    return Ok(addr);
                }
                Ok(Err(_)) | Err(_) => {
                    debug!("No pump at {}", addr);
                    continue;
                }
            }
        }
        Err(Error::new(
            ErrorKind::NotFound,
            format!(
                "No available gas pump found in range {}-{}",
                start_port, end_port
            ),
        ))
    }

    fn choose_liters_to_load(&self) -> Result<u32> {
        print!("Enter liters to load: ");
        stdout().flush()?;
        let mut liters = String::new();
        stdin().read_line(&mut liters)?;
        let liters: u32 = liters
            .trim()
            .parse()
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "Invalid liters value entered"))?;
        Ok(liters)
    }

    fn send_fuel_request(&self, card_addr: &Addr<Card>, liters: u32) {
        card_addr.do_send(FuelRequest::new(self.company_id, self.card_id, liters));
    }

    async fn waiting_for_charge_completion(&self, rx: &mut mpsc::UnboundedReceiver<FuelComplete>) {
        match rx.recv().await {
            Some(fuel_complete) => {
                if fuel_complete.success {
                    println!("✅ Charge completed successfully");
                } else {
                    eprintln!("❌ Charge failed");
                }
            }
            None => {
                debug!("Failed to receive charge completion notification");
            }
        }
    }

    async fn check_pending_result_broadcast(&self) -> Result<Option<(String, GasPumpYPFMessage)>> {
        print!("Do you have pending expenses? (yes/no): ");
        stdout().flush()?;
        let mut answer = String::new();
        stdin().read_line(&mut answer)?;
        let answer = answer.trim().to_lowercase();

        if answer != "yes" {
            return Ok(None);
        }

        // Usar los puertos calculados automáticamente
        let start_port = self.get_start_port();
        let end_port = self.get_end_port();
        let pump_addresses: Vec<String> = (start_port..=end_port)
            .map(|port| format!("127.0.0.1:{}", port))
            .collect();

        println!(
            "🔍 Checking for pending results in pumps {}-{}...",
            start_port, end_port
        );

        for pump_addr in pump_addresses {
            debug!("Querying pump {}...", pump_addr);

            match timeout(
                Duration::from_millis(500),
                tokio::net::TcpStream::connect(&pump_addr),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    let (mut reader, mut writer) = stream.into_split();

                    if let Err(e) = TCPProtocol::send_with_writer(
                        &mut writer,
                        MsgType::GetPendingResult,
                        &GetPendingCardResult {
                            card_id: self.card_id,
                        },
                    )
                    .await
                    {
                        debug!("Error sending to pump {}: {}", pump_addr, e);
                        continue;
                    }
                    drop(writer);

                    debug!("Waiting for response from pump {}...", pump_addr);

                    // First read the message type
                    let response_msg_type = match timeout(
                        Duration::from_secs(2),
                        TCPProtocol::receive_msg_type_from_reader(&mut reader),
                    )
                    .await
                    {
                        Ok(Ok(msg_type)) => msg_type,
                        Ok(Err(e)) => {
                            debug!("Error reading message type: {}", e);
                            continue;
                        }
                        Err(_) => {
                            debug!("Timeout waiting for message type");
                            continue;
                        }
                    };

                    match response_msg_type {
                        MsgType::FuelComplete => {
                            // We have a pending result
                            match timeout(
                                Duration::from_secs(2),
                                TCPProtocol::receive_msg_from_reader::<GasPumpYPFMessage>(
                                    &mut reader,
                                ),
                            )
                            .await
                            {
                                Ok(Ok(pending_msg)) => {
                                    println!("Received pending result: {:?}", pending_msg);
                                    return Ok(Some((pump_addr, pending_msg)));
                                }
                                Ok(Err(e)) => {
                                    debug!("Error reading pending result: {}", e);
                                }
                                Err(_) => {
                                    debug!("Timeout reading pending result");
                                }
                            }
                        }
                        MsgType::NoPendingResult => {
                            debug!("No pending result at pump {}", pump_addr);
                            // Read the payload to clear the stream, but ignore it
                            let _ = timeout(
                                Duration::from_secs(1),
                                TCPProtocol::receive_msg_from_reader::<GasPumpYPFMessage>(
                                    &mut reader,
                                ),
                            )
                            .await;
                        }
                        other => {
                            debug!("Unexpected message type: {:?}", other);
                        }
                    }

                    drop(reader);
                }
                Ok(Err(e)) => {
                    debug!("Failed to connect to pump {}: {}", pump_addr, e);
                }
                Err(_) => {
                    debug!("Timeout connecting to pump {}", pump_addr);
                }
            }
        }
        debug!("No pump has pending result for card {}", self.card_id);
        Ok(None)
    }
}
