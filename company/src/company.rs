use actix::{Actor, ActorFutureExt, Addr, AsyncContext, Context, Handler, Message, WrapFuture};
use common::{company_debug, company_error, company_info};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task;

use crate::persistence::*;
use crate::regional::{RegionalAdminHandler, SendPaymentProcessed};
use common::message::*;
use common::tcp_protocol::TCPProtocol;

#[derive(Message)]
#[rtype(result = "bool")]
pub struct ProcessPumpExpense {
    pub pump_address: String,
    pub expense: Expense,
    pub regional_admin_id: String,
    pub service_station_id: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PaymentProcessedAck {
    pub pump_address: String,
    pub expense: Expense,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct StoreRegionalAdminHandler {
    pub regional_admin_id: String,
    pub handler: Addr<RegionalAdminHandler>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ServiceStationReconnected {
    pub service_station_id: String,
    pub regional_admin_id: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct AttachCompanyAdmin {
    pub stream: TcpStream,
}

#[allow(dead_code)]
pub struct CompanyHandler {
    company_id: u32,
    reader: Option<OwnedReadHalf>,
    writer: Option<OwnedWriteHalf>,
    persistence: Addr<PersistenceActor>,
    balance: u32,
    cards: HashMap<u32, Card>,
    pending_confirmations: HashMap<String, PendingConfirmation>,
    regional_admin_handlers: HashMap<String, Addr<RegionalAdminHandler>>,
}

impl CompanyHandler {
    pub fn new(company_id: u32, persistence: Addr<PersistenceActor>) -> Self {
        let path = format!("data/{}.json", company_id);

        let mut balance = 0;
        let mut cards = HashMap::new();
        let mut pending_confirmations = HashMap::new();

        #[allow(clippy::collapsible_if)]
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str::<CompanyStateData>(&data) {
                company_debug!("[CompanyHandler {}] State restored by JSON", company_id);
                balance = state.balance;
                cards = state.cards;
                pending_confirmations = state.pending_confirmations;
            }
        }

        CompanyHandler {
            company_id,
            reader: None,
            writer: None,
            persistence,
            balance,
            cards,
            pending_confirmations,
            regional_admin_handlers: HashMap::new(),
        }
    }

    /// Generate a deterministic key for confirmation tracking
    /// Key format: "{pump_address}:{company_id}:{card_id}:{total}:{date}"
    fn make_confirmation_key(pump_address: &str, expense: &Expense) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            pump_address, expense.company_id, expense.card_id, expense.total, expense.date
        )
    }

    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn send_payment_processed_to_regional_admin(
        &mut self,
        _ctx: &mut Context<Self>,
        regional_id: &str,
        pump_address: &str,
        expense: &Expense,
        accepted: bool,
        service_station_id: &str,
    ) {
        if let Some(regional_handler) = self.regional_admin_handlers.get(regional_id) {
            company_debug!(
                "[CompanyHandler {}] Sending PaymentProcessed via RegionalAdminHandler {}: pump={}, accepted={}, total={}",
                self.company_id,
                regional_id,
                pump_address,
                accepted,
                expense.total
            );

            regional_handler.do_send(SendPaymentProcessed {
                pump_address: pump_address.to_string(),
                expense: expense.clone(),
                accepted,
                service_station_id: service_station_id.to_string(),
            });
        } else {
            company_error!(
                "[CompanyHandler {}] Cannot send PaymentProcessed: RegionalAdminHandler {} not found",
                self.company_id,
                regional_id
            );
        }
    }

    fn retry_pending_confirmations(&mut self, ctx: &mut Context<Self>) {
        const MIN_RETRY_AGE_SECS: u64 = 5;
        const MAX_RETRIES: u32 = 10;

        let current_time = Self::current_timestamp();
        let mut to_retry = Vec::new();
        let mut to_log_exceeded = Vec::new();

        for (key, confirmation) in self.pending_confirmations.iter() {
            let age_secs = current_time.saturating_sub(confirmation.timestamp);

            if age_secs >= MIN_RETRY_AGE_SECS {
                if confirmation.retry_count < MAX_RETRIES {
                    to_retry.push((key.clone(), confirmation.clone()));
                } else {
                    to_log_exceeded.push((key.clone(), confirmation.clone()));
                }
            }
        }

        let retry_count = to_retry.len();

        for (key, confirmation) in to_retry {
            if let Some(pending) = self.pending_confirmations.get_mut(&key) {
                pending.retry_count += 1;
                pending.timestamp = current_time;

                company_info!(
                    "[CompanyHandler {}] Retrying pending confirmation (attempt {}/{}): pump={}, expense_total={}",
                    self.company_id,
                    pending.retry_count,
                    MAX_RETRIES,
                    confirmation.pump_address,
                    confirmation.expense.total
                );

                // Send PaymentProcessed to Regional Admin
                self.send_payment_processed_to_regional_admin(
                    ctx,
                    &confirmation.regional_admin_id,
                    &confirmation.pump_address,
                    &confirmation.expense,
                    confirmation.accepted,
                    &confirmation.service_station_id,
                );
            }
        }

        if retry_count > 0 {
            self.persist_state();
        }
    }

    fn persist_state(&self) {
        let state = CompanyStateData::from_handler(
            self.balance,
            self.cards.clone(),
            self.pending_confirmations.clone(),
        );
        let serialized = serde_json::to_string_pretty(&state).unwrap();

        self.persistence.do_send(PersistCompanyState {
            company_id: self.company_id,
            json_state: serialized,
        });
    }

    fn spawn_read_loop(&mut self, ctx: &mut Context<Self>) {
        let mut reader = self.reader.take().expect("reader missing");
        let addr = ctx.address();

        task::spawn(async move {
            loop {
                let msg_type = match TCPProtocol::receive_msg_type_from_reader(&mut reader).await {
                    Ok(t) => t,
                    Err(_) => {
                        addr.do_send(Disconnect);
                        break;
                    }
                };

                let payload: serde_json::Value =
                    match TCPProtocol::receive_msg_from_reader(&mut reader).await {
                        Ok(v) => v,
                        Err(e) => {
                            company_error!("payload error: {}", e);
                            break;
                        }
                    };

                match msg_type {
                    MsgType::AskTotalBalance => {
                        addr.do_send(AskTotalBalance);
                    }

                    MsgType::ChargeBalance => {
                        let amount = payload["amount"].as_u64().unwrap_or(0) as u32;
                        addr.do_send(ChargeBalance { amount });
                    }

                    MsgType::RegisterNewCard => {
                        let card_id = payload["card_id"].as_str().unwrap_or("").to_string();
                        let amount = payload["amount"].as_u64().unwrap_or(0) as u32;
                        addr.do_send(RegisterNewCard { card_id, amount });
                    }

                    MsgType::CheckCardLimit => {
                        let card_id = payload["card_id"].as_str().unwrap_or("").to_string();
                        addr.do_send(CheckCardLimit { card_id });
                    }

                    MsgType::EstablishCardBalance => {
                        let card_id = payload["card_id"].as_str().unwrap_or("").to_string();
                        let amount = payload["amount"].as_u64().unwrap_or(0) as u32;
                        addr.do_send(EstablishCardBalance { card_id, amount });
                    }

                    MsgType::CheckAllCards => {
                        addr.do_send(CheckAllCards);
                    }

                    MsgType::CheckExpenses => {
                        addr.do_send(CheckExpenses);
                    }

                    _ => {
                        company_debug!(
                            "[CompanyHandler] Received an unknown MsgType: {:?}",
                            msg_type
                        );
                    }
                }
            }
        });
    }

    pub fn send_msg<T: serde::Serialize>(
        &mut self,
        ctx: &mut Context<Self>,
        msg_type: MsgType,
        payload: &T,
    ) {
        if let Some(mut writer) = self.writer.take() {
            let payload_owned = serde_json::to_value(payload).unwrap_or_else(|e| {
                company_error!("Failed to serialize payload: {}", e);
                serde_json::json!({ "error": "serialization failed" })
            });

            let fut = async move {
                let res =
                    TCPProtocol::send_with_writer(&mut writer, msg_type, &payload_owned).await;
                (writer, res)
            }
            .into_actor(self)
            .map(|(writer, res), actor, _ctx| {
                actor.writer = Some(writer);
                if let Err(e) = res {
                    company_error!("send error: {}", e);
                }
            });

            ctx.spawn(fut);
        }
    }

    pub fn add_balance(&mut self, amount: u32) -> Result<u32, String> {
        company_info!(
            "[CompanyHandler {}] Current balance: {}. Adding: {}",
            self.company_id,
            self.balance,
            amount
        );
        self.balance += amount;

        self.persist_state();
        Ok(self.balance)
    }

    pub fn create_new_card(&mut self, card_id: u32, initial_balance: u32) -> Result<u32, String> {
        if self.cards.contains_key(&card_id) {
            return Err(format!("Card ID {} already exists", card_id));
        }

        if self.balance < initial_balance {
            return Err(format!(
                "Cannot create card with initial balance: {}. Company balance is: {}",
                initial_balance, self.balance
            ));
        }

        company_info!(
            "[CompanyHandler {}] Registering new Card ID {} with initial balance: {}",
            self.company_id,
            card_id,
            initial_balance
        );

        self.cards.insert(
            card_id,
            Card {
                saldo_actual: initial_balance,
                gastos_recientes: Vec::new(),
            },
        );

        self.persist_state();
        Ok(initial_balance)
    }

    pub fn get_card_limit(&self, card_id: u32) -> Result<u32, String> {
        match self.cards.get(&card_id) {
            Some(card) => Ok(card.saldo_actual),

            None => {
                let error_msg = format!("Card ID {} not found.", card_id);
                Err(error_msg)
            }
        }
    }

    pub fn set_card_limit(&mut self, card_id: u32, mut new_limit: u32) -> Result<u32, String> {
        match self.cards.get_mut(&card_id) {
            Some(card) => {
                if new_limit > self.balance {
                    new_limit = self.balance;
                }
                card.saldo_actual = new_limit;

                self.persist_state();
                Ok(new_limit)
            }

            None => {
                let error_msg = format!(
                    "Card ID {} not found. Could not update the balance!",
                    card_id
                );
                Err(error_msg)
            }
        }
    }

    pub fn get_all_cards(&self) -> Vec<(u32, Card)> {
        self.cards
            .iter()
            .map(|(card_id, card)| (*card_id, card.clone()))
            .collect()
    }

    pub fn get_all_expenses(&self) -> Vec<Expense> {
        let mut consolidated_expenses: Vec<Expense> = Vec::new();

        for card in self.cards.values() {
            let cloned_expenses = card.gastos_recientes.iter().cloned();

            consolidated_expenses.extend(cloned_expenses);
        }
        consolidated_expenses
    }

    pub fn process_transaction(&mut self, new_gasto: Expense) -> Result<(), String> {
        let amount = new_gasto.total;

        company_info!(
            "[CompanyHandler {}] Expense received: card_id={}, total={}, date={}",
            self.company_id,
            new_gasto.card_id,
            amount,
            new_gasto.date
        );
        company_debug!(
            "[CompanyHandler {}] Current state: global_balance={}, registered_cards={}",
            self.company_id,
            self.balance,
            self.cards.len()
        );

        if amount == 0 {
            return Err(format!("Amount of transaction invalid (zero): {}", amount));
        }

        // if card is not registered, register it automatically
        let card = self.cards.entry(new_gasto.card_id).or_insert_with(|| {
            company_info!(
                "[CompanyHandler {}] → Card ID {} not found. Registering new card...",
                self.company_id,
                new_gasto.card_id
            );
            // design decision: init balance will be 10% of global balance
            let initial_balance = (self.balance as f32 * 0.10).floor() as u32;
            Card {
                saldo_actual: initial_balance,
                gastos_recientes: Vec::new(),
            }
        });

        company_debug!(
            "[CompanyHandler {}] Current state of Card ID {}: actual_balance={}",
            self.company_id,
            new_gasto.card_id,
            card.saldo_actual
        );

        // check per-card limit first
        if card.saldo_actual < amount {
            company_info!(
                "[CompanyHandler {}] Limit reached in Card ID {}: card_balance={}, price={}",
                self.company_id,
                new_gasto.card_id,
                card.saldo_actual,
                amount
            );

            return Err(format!(
                "Limit reached! Card ID {} has only {} available (requested {}).",
                new_gasto.card_id, card.saldo_actual, amount
            ));
        }

        // check company global balance
        if self.balance < amount {
            company_debug!(
                "[CompanyHandler {}] Company global balance insufficient: global_balance={}, price={}",
                self.company_id,
                self.balance,
                amount
            );
            return Err(format!(
                "Company global balance insufficient: company_balance={}, requested={}",
                self.balance, amount
            ));
        }

        card.saldo_actual -= amount;
        self.balance -= amount;
        card.gastos_recientes.push(new_gasto);

        company_info!(
            "[CompanyHandler {}] Final state after transaction: global_balance={}, card_balance={}",
            self.company_id,
            self.balance,
            card.saldo_actual
        );

        self.persist_state();
        Ok(())
    }
}

impl Actor for CompanyHandler {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        company_info!("[CompanyHandler {}] Actor started.", self.company_id);

        ctx.run_interval(std::time::Duration::from_secs(5), |act, ctx| {
            act.retry_pending_confirmations(ctx);
        });
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        company_info!(
            "[CompanyHandler {}] Actor stopped! Final Balance: {}. Cards: {}.",
            self.company_id,
            self.balance,
            self.cards.len()
        );
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect;

impl Handler<Disconnect> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, _msg: Disconnect, _ctx: &mut Self::Context) {
        company_info!(
            "[CompanyHandler {}] CompanyAdmin disconnected! Closing TCP connection...",
            self.company_id
        );

        self.reader = None;
        self.writer = None;
    }
}

impl Handler<AskTotalBalance> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, _msg: AskTotalBalance, ctx: &mut Self::Context) {
        let payload = serde_json::json!({
            "saldo_total": self.balance
        });

        self.send_msg(ctx, MsgType::AskTotalBalance, &payload);
    }
}

impl Handler<ChargeBalance> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: ChargeBalance, ctx: &mut Self::Context) {
        let amount = msg.amount;
        let new_balance = match self.add_balance(amount) {
            Ok(new) => new,
            Err(e) => {
                company_error!(
                    "[CompanyHandler {}] Error in add_balance: {}",
                    self.company_id,
                    e
                );
                self.balance
            }
        };
        let payload = serde_json::json!({
            "nuevo_saldo": new_balance
        });
        self.send_msg(ctx, MsgType::ChargeBalance, &payload);
    }
}

impl Handler<RegisterNewCard> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: RegisterNewCard, ctx: &mut Self::Context) {
        let id = msg.card_id.parse::<u32>().unwrap_or(0);
        let res = self.create_new_card(id, msg.amount);

        let payload = match res {
            Ok(init_balance) => serde_json::json!({
                "tarjeta_id": id,
                "saldo_inicial": init_balance
            }),
            Err(err_msg) => serde_json::json!({
                "error": err_msg
            }),
        };

        self.send_msg(ctx, MsgType::RegisterNewCard, &payload);
    }
}

impl Handler<CheckCardLimit> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: CheckCardLimit, ctx: &mut Self::Context) {
        let id = msg.card_id.parse::<u32>().unwrap_or(0);
        let res = self.get_card_limit(id);

        let payload = match res {
            Ok(limit) => serde_json::json!({
                "card_id": id,
                "limite": limit
            }),
            Err(err_msg) => serde_json::json!({
                "error": err_msg
            }),
        };

        self.send_msg(ctx, MsgType::CheckCardLimit, &payload);
    }
}

impl Handler<EstablishCardBalance> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: EstablishCardBalance, ctx: &mut Self::Context) {
        let id = msg.card_id.parse::<u32>().unwrap_or(0);
        let res = self.set_card_limit(id, msg.amount);

        let payload = match res {
            Ok(new_limit) => serde_json::json!({
                "tarjeta_id": id,
                "nuevo_saldo": new_limit
            }),
            Err(err_msg) => serde_json::json!({
                "error": err_msg
            }),
        };

        self.send_msg(ctx, MsgType::EstablishCardBalance, &payload);
    }
}

impl Handler<CheckAllCards> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, _msg: CheckAllCards, ctx: &mut Self::Context) {
        let tarjetas: Vec<_> = self
            .get_all_cards()
            .into_iter()
            .map(|(card_id, card)| {
                serde_json::json!({
                    "card_id": card_id,
                    "saldo_actual": card.saldo_actual,
                    "gastos_recientes": card.gastos_recientes.len()
                })
            })
            .collect();

        let payload = serde_json::json!({
            "cantidad_tarjetas": tarjetas.len(),
            "tarjetas": tarjetas
        });

        self.send_msg(ctx, MsgType::CheckAllCards, &payload);
    }
}

impl Handler<CheckExpenses> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, _msg: CheckExpenses, ctx: &mut Self::Context) {
        let gastos: Vec<_> = self
            .get_all_expenses()
            .into_iter()
            .map(|g| {
                serde_json::json!({
                    "card_id": g.card_id,
                    "company_id": g.company_id,
                    "monto": g.total,
                    "date": g.date
                })
            })
            .collect();

        let payload = serde_json::json!({
            "gastos": gastos,
        });

        self.send_msg(ctx, MsgType::CheckExpenses, &payload);
    }
}

impl Handler<ProcessPumpExpense> for CompanyHandler {
    type Result = bool;

    fn handle(&mut self, msg: ProcessPumpExpense, _ctx: &mut Self::Context) -> Self::Result {
        company_debug!(
            "[CompanyHandler {}] → Processing Expense received from pump {} via RegionalAdmin {}...",
            self.company_id,
            msg.pump_address,
            msg.regional_admin_id
        );

        let accepted = self.process_transaction(msg.expense.clone()).is_ok();

        if accepted {
            company_info!(
                "[CompanyHandler {}] Transaction final result: SUCCESS",
                self.company_id
            );
        } else {
            company_info!(
                "[CompanyHandler {}] Transaction final result: FAILED",
                self.company_id
            );
        }

        // Add to pending confirmations (WAL-style persistence)
        let key = Self::make_confirmation_key(&msg.pump_address, &msg.expense);
        let pending_confirmation = PendingConfirmation {
            pump_address: msg.pump_address.clone(),
            expense: msg.expense.clone(),
            accepted,
            timestamp: Self::current_timestamp(),
            retry_count: 0,
            regional_admin_id: msg.regional_admin_id.clone(),
            service_station_id: msg.service_station_id.clone(),
        };

        company_debug!(
            "[CompanyHandler {}] Adding pending confirmation: key={}, service_station={}, accepted={}",
            self.company_id,
            key,
            msg.service_station_id,
            accepted
        );

        self.pending_confirmations.insert(key, pending_confirmation);
        self.persist_state();

        self.send_payment_processed_to_regional_admin(
            _ctx,
            &msg.regional_admin_id,
            &msg.pump_address,
            &msg.expense,
            accepted,
            &msg.service_station_id,
        );

        company_debug!(
            "[CompanyHandler {}] Pending confirmation persisted and PaymentProcessed sent, awaiting ACK from Regional Admin",
            self.company_id
        );

        accepted
    }
}

impl Handler<PaymentProcessedAck> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: PaymentProcessedAck, _ctx: &mut Self::Context) -> Self::Result {
        let key = Self::make_confirmation_key(&msg.pump_address, &msg.expense);

        if let Some(confirmation) = self.pending_confirmations.remove(&key) {
            company_info!(
                "[CompanyHandler {}] Received ACK for pending confirmation: key={}, pump={}, expense_total={}, retry_count={}",
                self.company_id,
                key,
                msg.pump_address,
                msg.expense.total,
                confirmation.retry_count
            );

            // Persist the removal
            self.persist_state();
        } else {
            company_debug!(
                "[CompanyHandler {}] Received ACK for unknown/already-processed confirmation: key={}, pump={}",
                self.company_id,
                key,
                msg.pump_address
            );
        }
    }
}

impl Handler<StoreRegionalAdminHandler> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: StoreRegionalAdminHandler, _ctx: &mut Self::Context) -> Self::Result {
        company_info!(
            "[CompanyHandler {}] Storing RegionalAdminHandler for regional_admin_id={}",
            self.company_id,
            msg.regional_admin_id
        );
        self.regional_admin_handlers
            .insert(msg.regional_admin_id, msg.handler);
    }
}

impl Handler<ServiceStationReconnected> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: ServiceStationReconnected, ctx: &mut Self::Context) -> Self::Result {
        company_info!(
            "[CompanyHandler {}] Service station {} reconnected to new regional admin {}",
            self.company_id,
            msg.service_station_id,
            msg.regional_admin_id
        );

        // Find all pending confirmations for this service station
        let mut to_retry = Vec::new();
        for (key, confirmation) in self.pending_confirmations.iter() {
            if confirmation.service_station_id == msg.service_station_id {
                to_retry.push((key.clone(), confirmation.clone()));
            }
        }

        if to_retry.is_empty() {
            company_debug!(
                "[CompanyHandler {}] No pending confirmations found for service station {}",
                self.company_id,
                msg.service_station_id
            );
            return;
        }

        company_info!(
            "[CompanyHandler {}] Found {} pending confirmations for service station {}, retrying via new regional admin {}",
            self.company_id,
            to_retry.len(),
            msg.service_station_id,
            msg.regional_admin_id
        );

        // Update all matching pending confirmations with new regional admin ID and retry
        let current_time = Self::current_timestamp();
        for (key, confirmation) in to_retry {
            if let Some(pending) = self.pending_confirmations.get_mut(&key) {
                pending.regional_admin_id = msg.regional_admin_id.clone();
                pending.retry_count += 1;
                pending.timestamp = current_time;

                company_debug!(
                    "[CompanyHandler {}] Retrying confirmation via new regional admin: pump={}, expense_total={}, attempt={}",
                    self.company_id,
                    confirmation.pump_address,
                    confirmation.expense.total,
                    pending.retry_count
                );

                self.send_payment_processed_to_regional_admin(
                    ctx,
                    &msg.regional_admin_id,
                    &confirmation.pump_address,
                    &confirmation.expense,
                    confirmation.accepted,
                    &msg.service_station_id,
                );
            }
        }

        self.persist_state();
    }
}

impl Handler<AttachCompanyAdmin> for CompanyHandler {
    type Result = ();

    fn handle(&mut self, msg: AttachCompanyAdmin, ctx: &mut Context<Self>) {
        let (reader, writer) = msg.stream.into_split();
        self.reader = Some(reader);
        self.writer = Some(writer);

        company_info!(
            "[CompanyHandler {}] CompanyAdmin connected (attaching TCP)",
            self.company_id
        );

        self.spawn_read_loop(ctx);
    }
}

// ==========================================
// ==========      TESTS      ===============
// ==========================================
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    #[allow(dead_code)]
    pub struct CompanyHandlerTesting {
        pub company_id: u32,
        pub balance: u32,
        pub cards: std::collections::HashMap<u32, Card>,
    }

    impl CompanyHandlerTesting {
        pub fn process_transaction(&mut self, new_gasto: Expense) -> Result<(), String> {
            let amount = new_gasto.total;

            if amount == 0 || amount > self.balance {
                return Err(format!("Amount of transaction invalid: {}", amount));
            }

            // Si la tarjeta no existe, la creamos automáticamente
            let card = self.cards.entry(new_gasto.card_id).or_insert_with(|| {
                Card {
                    saldo_actual: self.balance, // o 0, según lógica
                    gastos_recientes: Vec::new(),
                }
            });

            if card.saldo_actual < amount {
                return Err(format!(
                    "Limit reached! You can't spend more than {} !",
                    card.saldo_actual
                ));
            }

            card.saldo_actual -= amount;
            self.balance -= amount;
            card.gastos_recientes.push(new_gasto);

            Ok(())
        }
    }

    fn make_handler(balance: u32, cards: HashMap<u32, Card>) -> CompanyHandlerTesting {
        CompanyHandlerTesting {
            company_id: 1,
            balance,
            cards,
        }
    }

    fn mk_expense(card: u32, total: u32) -> Expense {
        Expense {
            card_id: card,
            company_id: 1,
            total,
            date: "2024-01-01".to_string(),
        }
    }

    #[test]
    fn test_transaction_success() {
        let mut handler = make_handler(
            1000,
            HashMap::from([(
                10,
                Card {
                    saldo_actual: 500,
                    gastos_recientes: vec![],
                },
            )]),
        );

        let res = handler.process_transaction(mk_expense(10, 200));
        assert!(res.is_ok());

        assert_eq!(handler.balance, 800);
        assert_eq!(handler.cards[&10].saldo_actual, 300);
        assert_eq!(handler.cards[&10].gastos_recientes.len(), 1);
    }

    #[test]
    fn test_transaction_creates_card_if_missing() {
        let mut handler = make_handler(1000, HashMap::new());

        let res = handler.process_transaction(mk_expense(55, 100));
        assert!(res.is_ok());

        assert!(handler.cards.contains_key(&55));
        let card = &handler.cards[&55];

        assert_eq!(handler.balance, 900);
        assert_eq!(card.saldo_actual, 900); // 1000 - 100
        assert_eq!(card.gastos_recientes.len(), 1);
    }

    #[test]
    fn test_transaction_limit_exceeded() {
        let mut handler = make_handler(
            1000,
            HashMap::from([(
                3,
                Card {
                    saldo_actual: 50,
                    gastos_recientes: vec![],
                },
            )]),
        );

        let res = handler.process_transaction(mk_expense(3, 200));

        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            "Limit reached! You can't spend more than 50 !"
        );
    }

    #[test]
    fn test_transaction_global_balance_insufficient() {
        let mut handler = make_handler(100, HashMap::new());

        let res = handler.process_transaction(mk_expense(9, 200));

        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Amount of transaction invalid: 200");
    }

    #[test]
    fn test_transaction_amount_zero_invalid() {
        let mut handler = make_handler(1000, HashMap::new());

        let res = handler.process_transaction(mk_expense(7, 0));

        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Amount of transaction invalid: 0");
    }
}
