use std::collections::HashMap;

use actix::prelude::*;
use common::message::Expense;
use common::{company_debug, company_info};
use serde::{Deserialize, Serialize};
use tokio::fs;

// ----------------------------------------------------
// Struct for represent a Card
// (This is going to be the value on HashMap)
// ----------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub saldo_actual: u32,              // El saldo disponible de la tarjeta
    pub gastos_recientes: Vec<Expense>, // Lista de gastos asociados a esta tarjeta
}

// ----------------------------------------------------
// Struct for pending confirmation (WAL entry)
// ----------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingConfirmation {
    pub pump_address: String,
    pub expense: Expense,
    pub accepted: bool,
    pub timestamp: u64, // Unix timestamp in seconds
    pub retry_count: u32,
    pub regional_admin_id: String,
    pub service_station_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct CompanyStateData {
    pub balance: u32,
    pub cards: HashMap<u32, Card>,
    #[serde(default)]
    pub pending_confirmations: HashMap<String, PendingConfirmation>,
}

impl CompanyStateData {
    pub fn from_handler(
        balance: u32,
        cards: HashMap<u32, Card>,
        pending_confirmations: HashMap<String, PendingConfirmation>,
    ) -> Self {
        Self {
            balance,
            cards,
            pending_confirmations,
        }
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PersistCompanyState {
    pub company_id: u32,
    pub json_state: String,
}

/// Actor who persists the state of each Company in a JSON file
pub struct PersistenceActor;

impl Actor for PersistenceActor {
    type Context = Context<Self>;
}

impl Handler<PersistCompanyState> for PersistenceActor {
    type Result = ();

    fn handle(&mut self, msg: PersistCompanyState, _ctx: &mut Self::Context) {
        let file_path = format!("data/{}.json", msg.company_id);

        actix_rt::spawn(async move {
            // create folder if not exists
            let _ = fs::create_dir_all("data").await;

            if let Err(e) = fs::write(&file_path, msg.json_state).await {
                company_debug!("[Persistence] ERROR writing {}: {}", file_path, e);
            } else {
                company_info!("[Persistence] Saved {} !", file_path);
            }
        });
    }
}
