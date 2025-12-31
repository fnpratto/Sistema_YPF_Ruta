use crate::admin_actor::CompanyAdminActor;
use actix::{Actor, System};
use common::constants::{COMPANY_LEADER_PORT, LOCALHOST};
use common::message::*;
use common::{company_debug, company_error, company_info};

use std::io::{Write, stdin, stdout};
use tokio::io::AsyncWriteExt;
use tokio::{net::TcpStream, sync::mpsc};

pub struct CompanyAdminClient {
    pub company_id: u32,
}

impl CompanyAdminClient {
    pub fn new(company_id: u32) -> Self {
        Self { company_id }
    }

    pub async fn run(&self) -> std::io::Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();

        let server_address = format!("{}:{}", LOCALHOST, COMPANY_LEADER_PORT);
        let stream = match TcpStream::connect(&server_address).await {
            Ok(s) => {
                company_info!("[CompanyAdmin] Connected to server!");
                s
            }
            Err(e) => {
                company_debug!("Failed to connect to server at {}: {}", server_address, e);
                company_debug!("Terminating program - cannot proceed without server connection");
                System::current().stop();
                std::process::exit(1);
            }
        };

        let (reader, mut writer) = stream.into_split();
        let id_message = format!("COMPANY {}\n", self.company_id);

        let _ = writer.write_all(id_message.as_bytes()).await;
        let _ = writer.flush().await;

        let actor = CompanyAdminActor::new(writer, reader, self.company_id, tx);
        let actor_addr = actor.start();

        loop {
            let choice = self.menu()?;
            match choice.as_str() {
                "1" => actor_addr.do_send(AskTotalBalance),
                "2" => {
                    let amount = self.input_number("Monto a cargar: ")?;
                    actor_addr.do_send(ChargeBalance { amount });
                }
                "3" => {
                    let card_id = self.input_string("ID Tarjeta: ")?;
                    let amount = self.input_number("Saldo Inicial: ")?;
                    actor_addr.do_send(RegisterNewCard { card_id, amount });
                }
                "4" => {
                    let card = self.input_string("ID Tarjeta: ")?;
                    actor_addr.do_send(CheckCardLimit { card_id: card });
                }
                "5" => {
                    let card_id = self.input_string("ID Tarjeta: ")?;
                    let amount = self.input_number("Nuevo saldo: ")?;
                    actor_addr.do_send(EstablishCardBalance { card_id, amount });
                }
                "6" => {
                    actor_addr.do_send(CheckAllCards);
                }
                "7" => {
                    actor_addr.do_send(CheckExpenses);
                }
                "0" => {
                    println!("Saliendo...");
                    break;
                }
                _ => {
                    println!("Opción inválida");
                    continue;
                }
            }

            self.waiting_for_server_response(&mut rx).await;
        }

        drop(actor_addr);
        Ok(())
    }

    async fn waiting_for_server_response(&self, rx: &mut mpsc::UnboundedReceiver<String>) {
        if let Some(resp) = rx.recv().await {
            println!("\n=> Server Response: ");
            let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();
            let pretty_resp = serde_json::to_string_pretty(&parsed).unwrap();
            println!("{}", pretty_resp);
        } else {
            company_error!("Error receiving server response!");
        }
    }

    fn menu(&self) -> std::io::Result<String> {
        println!();
        println!("=== Company Admin ===");
        println!("1. Chequear Saldo Global");
        println!("2. Cargar Saldo Global");
        println!("3. Registrar Nueva Tarjeta");
        println!("4. Chequear Saldo de Tarjeta");
        println!("5. Establecer Saldo de Tarjeta");
        println!("6. Visualizar Tarjetas Registradas");
        println!("7. Visualizar Gastos Recientes");
        println!("0. Salir");

        print!("Opción: ");
        stdout().flush()?;

        let mut s = String::new();
        stdin().read_line(&mut s)?;
        Ok(s.trim().to_string())
    }

    fn input_number(&self, txt: &str) -> std::io::Result<u32> {
        print!("{}", txt);
        stdout().flush()?;
        let mut s = String::new();
        stdin().read_line(&mut s)?;
        Ok(s.trim().parse().unwrap_or(0))
    }

    fn input_string(&self, txt: &str) -> std::io::Result<String> {
        print!("{}", txt);
        stdout().flush()?;
        let mut s = String::new();
        stdin().read_line(&mut s)?;
        Ok(s.trim().to_string())
    }
}
