use crate::message::StoreCompanyConnection;
use actix::{Actor, Addr, AsyncContext, Context, Handler};
use common::constants::{COMPANY_LEADER_PORT, LOCALHOST, get_province_name};
use common::message::{Expense, MsgType, RegionalAdminMessage};
use common::tcp_protocol::TCPProtocol;
use common::{crash_info, election_debug, election_info, fuel_debug, fuel_info, leader_info};
use log::debug;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::connection_actor::RegionalAdminConnectionActor;
use crate::message::{
    CompanyPaymentProcessed, PumpConnection, PumpExpense, SendPaymentProcessedToPump, SendToClient,
    SendToCompanyFailed, SendToPumpFailed, SendToPumpSuccess,
};

#[derive(Clone, Debug)]
pub enum PendingMessageType {
    PaymentProcessed {
        pump_address: String,
        expense: Expense,
        accepted: bool,
    },
}

#[derive(Clone, Debug)]
pub struct PendingMessage {
    pub msg_type: MsgType,
    pub message: PendingMessageType,
    pub timestamp: u64,
}

/*
 * RegionalAdmin Actor:
 *     - Listens for incoming TCP connections from gas pumps (SERVER role).
 *     - Connects to Company Leader(s) to forward expenses (CLIENT role).
 *     - Maintains mappings:
 *       - pump_address -> ConnectionActor (for responses to pumps)
 *       - company_id -> ConnectionActor (for forwarding to companies)
 *     - Buffers messages in-memory for disconnected pumps (indexed by pump_address and msg_type)
 *     - NO DISK PERSISTENCE (WAL is Company's responsibility)
 */
#[allow(dead_code)]
pub struct RegionalAdmin {
    address: String,
    id: u32,
    service_station_connections: HashMap<String, Addr<RegionalAdminConnectionActor>>, // service_station_id -> leader connection actor
    company_connection: Option<Addr<RegionalAdminConnectionActor>>,
    pending_messages: HashMap<String, Vec<PendingMessage>>, // Indexed by service_station_id
    pending_company_messages: Vec<RegionalAdminMessage>, // Buffer for unsent messages to the company
}

impl RegionalAdmin {
    pub fn new(province_id: u32, address: String) -> Self {
        let province_name = get_province_name(province_id).unwrap_or("Unknown");
        election_info!(
            "Creating RegionalAdmin for {} (ID: {}) on {}",
            province_name,
            province_id,
            address
        );
        Self {
            address,
            id: province_id,
            service_station_connections: HashMap::new(),
            company_connection: None,
            pending_messages: HashMap::new(),
            pending_company_messages: Vec::new(),
        }
    }

    pub fn start_pump_listener(&self, ctx: &mut Context<Self>) {
        let listen_address = self.address.clone();
        let admin_addr = ctx.address();
        let province_id = self.id;
        let province_name = get_province_name(province_id).unwrap_or("Unknown");

        election_info!(
            "RegionalAdmin for {} (ID: {}) starting pump listener on {}",
            province_name,
            province_id,
            listen_address
        );

        actix::spawn(async move {
            let listener = match TcpListener::bind(&listen_address).await {
                Ok(l) => {
                    leader_info!(
                        "RegionalAdmin for {} (ID: {}) listening for gas pumps on: {}",
                        province_name,
                        province_id,
                        listen_address
                    );
                    l
                }
                Err(e) => {
                    crash_info!("Failed to bind TcpListener on {}: {}", listen_address, e);
                    return;
                }
            };

            loop {
                match listener.accept().await {
                    Ok((stream, peer_address)) => {
                        fuel_info!(
                            "RegionalAdmin for {} (ID: {}) accepted connection from: {}",
                            province_name,
                            province_id,
                            peer_address
                        );

                        let (read_half, write_half) = stream.into_split();
                        let mut buf_reader = tokio::io::BufReader::new(read_half);
                        let mut handshake = String::new();

                        match buf_reader.read_line(&mut handshake).await {
                            Ok(_) => {
                                // Handshake format: "PUMP <address> <service_station_id>"
                                //               or: "COMPANY <address>"
                                let parts: Vec<&str> = handshake.split_whitespace().collect();

                                if parts.is_empty() {
                                    crash_info!(
                                        "❌ RegionalAdmin for {} (ID: {}) received empty handshake from {}.",
                                        province_name,
                                        province_id,
                                        peer_address
                                    );
                                    continue;
                                }

                                if parts[0] == "PUMP" {
                                    if parts.len() < 3 {
                                        crash_info!(
                                            "❌ RegionalAdmin for {} (ID: {}) received invalid handshake from {}: {}",
                                            province_name,
                                            province_id,
                                            peer_address,
                                            handshake.trim()
                                        );
                                        continue;
                                    }

                                    let pump_address = parts[1].to_string();
                                    let service_station_id = parts[2].to_string();

                                    fuel_info!(
                                        "✅ RegionalAdmin for {} (ID: {}) received handshake from pump: {} (service_station: {})",
                                        province_name,
                                        province_id,
                                        pump_address,
                                        service_station_id
                                    );

                                    let reader = buf_reader.into_inner();
                                    let conn_actor = RegionalAdminConnectionActor::start(
                                        reader,
                                        write_half,
                                        admin_addr.clone(),
                                        pump_address.clone(),
                                    );

                                    admin_addr.do_send(PumpConnection {
                                        pump_address,
                                        service_station_id,
                                        connection: conn_actor,
                                    });
                                } else if parts[0] == "COMPANY" {
                                    if parts.len() < 2 {
                                        crash_info!(
                                            "❌ RegionalAdmin for {} (ID: {}) received invalid handshake from {}: {}",
                                            province_name,
                                            province_id,
                                            peer_address,
                                            handshake.trim()
                                        );
                                        continue;
                                    }
                                    let company_address = parts[1].to_string();

                                    fuel_info!(
                                        "✅ RegionalAdmin for {} (ID: {}) received handshake from Company Leader (Server): {}",
                                        province_name,
                                        province_id,
                                        company_address,
                                    );

                                    let reader = buf_reader.into_inner();
                                    let conn_actor = RegionalAdminConnectionActor::start(
                                        reader,
                                        write_half,
                                        admin_addr.clone(),
                                        company_address.clone(),
                                    );

                                    admin_addr.do_send(StoreCompanyConnection {
                                        connection: conn_actor,
                                    });
                                } else {
                                    crash_info!(
                                        "❌ RegionalAdmin for {} (ID: {}) received unknown handshake type from {}: {}",
                                        province_name,
                                        province_id,
                                        peer_address,
                                        handshake.trim()
                                    );
                                    continue;
                                }
                            }
                            Err(e) => {
                                crash_info!(
                                    "❌ RegionalAdmin for {} (ID: {}) failed to read handshake from {}: {}",
                                    province_name,
                                    province_id,
                                    peer_address,
                                    e
                                );
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        crash_info!(
                            "RegionalAdmin for {} (ID: {}) failed to accept connection: {}",
                            province_name,
                            province_id,
                            e
                        );
                        continue;
                    }
                }
            }
        });
    }

    pub fn connect_to_company_leader(&self, ctx: &mut Context<Self>) {
        let admin_addr = ctx.address();
        let province_id = self.id;
        let province_name = get_province_name(province_id).unwrap_or("Unknown");

        let company_address = format!("{}:{}", LOCALHOST, COMPANY_LEADER_PORT);
        election_info!(
            "RegionalAdmin for {} (ID: {}) connecting to Company Leader at {}",
            province_name,
            province_id,
            company_address
        );

        actix::spawn(async move {
            match TCPProtocol::new(&company_address).await {
                Ok(company_leader_tcp_protocol) => {
                    leader_info!(
                        "RegionalAdmin for {} (ID: {}) connected to Company Leader at {}",
                        province_name,
                        province_id,
                        company_address,
                    );
                    let (reader, mut writer) = company_leader_tcp_protocol.into_split();

                    let handshake = format!("REGIONAL {}\n", province_id);
                    if let Err(e) = writer.write_all(handshake.as_bytes()).await {
                        crash_info!(
                            "RegionalAdmin for {} (ID: {}) failed to send handshake to Company Leader: {}",
                            province_name,
                            province_id,
                            e
                        );
                        return;
                    }
                    if let Err(e) = writer.flush().await {
                        crash_info!(
                            "RegionalAdmin for {} (ID: {}) failed to flush handshake to Company Leader: {}",
                            province_name,
                            province_id,
                            e
                        );
                        return;
                    }

                    election_debug!(
                        "RegionalAdmin for {} (ID: {}) sent handshake to Company Leader",
                        province_name,
                        province_id
                    );

                    let conn_actor = RegionalAdminConnectionActor::start(
                        reader,
                        writer,
                        admin_addr.clone(),
                        company_address.clone(),
                    );

                    admin_addr.do_send(StoreCompanyConnection {
                        connection: conn_actor,
                    });
                }
                Err(e) => {
                    crash_info!(
                        "RegionalAdmin for {} (ID: {}) failed to connect to Company Leader at {}: {}",
                        province_name,
                        province_id,
                        company_address,
                        e
                    );
                }
            }
        });
    }

    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn retry_pending_deliveries(&mut self, _ctx: &mut Context<Self>) {
        let stations_with_pending: Vec<String> = self.pending_messages.keys().cloned().collect();

        for service_station_id in stations_with_pending {
            #[allow(clippy::collapsible_if)]
            if let Some(station_conn_actor) =
                self.service_station_connections.get(&service_station_id)
            {
                if let Some(pending_msgs) = self.pending_messages.remove(&service_station_id) {
                    fuel_info!(
                        "📤 RegionalAdmin {} delivering {} buffered messages to reconnected service_station '{}'",
                        self.id,
                        pending_msgs.len(),
                        service_station_id
                    );

                    for pending_msg in pending_msgs {
                        match pending_msg.message {
                            PendingMessageType::PaymentProcessed {
                                pump_address,
                                expense,
                                accepted,
                            } => {
                                station_conn_actor.do_send(SendPaymentProcessedToPump {
                                    pump_address,
                                    expense,
                                    accepted,
                                    service_station_id: service_station_id.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn buffer_msg(&mut self, service_station_id: &str, pending_msg: PendingMessage) {
        #[allow(clippy::unwrap_or_default)]
        self.pending_messages
            .entry(service_station_id.to_string())
            .or_insert_with(Vec::new)
            .push(pending_msg);

        fuel_debug!(
            "📦 RegionalAdmin {} buffered message for service_station '{}' (total buffered: {})",
            self.id,
            service_station_id,
            self.pending_messages
                .get(service_station_id)
                .map_or(0, |v| v.len())
        );
    }

    fn send_ack_to_company(&self, pump_address: &str, expense: &Expense, msg_type: MsgType) {
        if let Some(company_conn_actor) = &self.company_connection {
            match msg_type {
                MsgType::PaymentProcessed => {
                    let ack_msg = RegionalAdminMessage::PaymentProcessedAck {
                        pump_address: pump_address.to_string(),
                        expense: expense.clone(),
                    };
                    company_conn_actor
                        .do_send(SendToClient::new(MsgType::PaymentProcessedAck, ack_msg));
                    fuel_debug!(
                        "✅ RegionalAdmin {} sent ACK to Company for pump '{}' expense ${}",
                        self.id,
                        pump_address,
                        expense.total
                    );
                }
                _ => {
                    fuel_debug!("No ACK needed for msg_type {:?}", msg_type);
                }
            }
        }
    }

    fn resend_pending_company_messages(&mut self) {
        if let Some(company_conn_actor) = &self.company_connection {
            for msg in self.pending_company_messages.drain(..) {
                company_conn_actor.do_send(SendToClient::new(MsgType::PumpExpense, msg));
                fuel_info!(
                    "📤 RegionalAdmin {} resent buffered message to Company Leader",
                    self.id
                );
            }
        }
    }
}

impl Actor for RegionalAdmin {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let province_name = get_province_name(self.id).unwrap_or("Unknown");
        leader_info!(
            "RegionalAdmin for {} (ID: {}) actor started",
            province_name,
            self.id
        );
        self.connect_to_company_leader(ctx);
        self.start_pump_listener(ctx);
        ctx.run_interval(std::time::Duration::from_secs(5), |act, ctx| {
            act.retry_pending_deliveries(ctx);
        });
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        let province_name = get_province_name(self.id).unwrap_or("Unknown");
        crash_info!(
            "RegionalAdmin for {} (ID: {}) actor stopped",
            province_name,
            self.id
        );
    }
}

impl Handler<PumpConnection> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: PumpConnection, _ctx: &mut Self::Context) -> Self::Result {
        fuel_info!(
            "📡 RegionalAdmin {} storing leader connection for service_station: {} (pump: {})",
            self.id,
            msg.service_station_id,
            msg.pump_address
        );

        let is_reconnection = self
            .service_station_connections
            .contains_key(&msg.service_station_id);

        if let Some(old_conn) = self
            .service_station_connections
            .insert(msg.service_station_id.clone(), msg.connection.clone())
        {
            fuel_info!(
                "🔄 RegionalAdmin {} replaced leader connection for service_station {} (new leader: {})",
                self.id,
                msg.service_station_id,
                msg.pump_address
            );
            drop(old_conn);
        }

        if let Some(company_conn_actor) = &self.company_connection {
            let regional_admin_id: String = format!("{}", self.id);

            let reconnect_msg = RegionalAdminMessage::ServiceStationReconnected {
                service_station_id: msg.service_station_id.clone(),
                regional_admin_id,
            };

            company_conn_actor.do_send(SendToClient::new(
                MsgType::ServiceStationReconnected,
                reconnect_msg,
            ));

            if is_reconnection {
                fuel_info!(
                    "📢 RegionalAdmin {} notified Company about service_station '{}' RECONNECTION",
                    self.id,
                    msg.service_station_id
                );
            } else {
                fuel_info!(
                    "📢 RegionalAdmin {} notified Company about service_station '{}' FIRST CONNECTION",
                    self.id,
                    msg.service_station_id
                );
            }
        }

        election_debug!(
            "RegionalAdmin {} now has {} service station leader connections",
            self.id,
            self.service_station_connections.len()
        );

        if let Some(pending_msgs) = self.pending_messages.remove(&msg.service_station_id) {
            fuel_info!(
                "📤 RegionalAdmin {} delivering {} buffered messages to reconnected service_station '{}' leader ({})",
                self.id,
                pending_msgs.len(),
                msg.service_station_id,
                msg.pump_address
            );

            for pending_msg in pending_msgs {
                match pending_msg.message {
                    PendingMessageType::PaymentProcessed {
                        pump_address,
                        expense,
                        accepted,
                    } => {
                        msg.connection.do_send(SendPaymentProcessedToPump {
                            pump_address,
                            expense,
                            accepted,
                            service_station_id: msg.service_station_id.clone(),
                        });
                    }
                }
            }
        }
    }
}

impl Handler<StoreCompanyConnection> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: StoreCompanyConnection, _ctx: &mut Self::Context) -> Self::Result {
        leader_info!("🏢 Storing connection to Company Leader");
        self.company_connection = Some(msg.connection);
        self.resend_pending_company_messages(); // Resend buffered messages upon reconnection
    }
}

impl Handler<PumpExpense> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: PumpExpense, _ctx: &mut Self::Context) -> Self::Result {
        fuel_info!(
            "💰 RegionalAdmin {} processing expense from pump {} (service_station: {}): company_id={}, card_id={}, total=${}, date={}",
            self.id,
            msg.pump_address,
            msg.service_station_id,
            msg.expense.company_id,
            msg.expense.card_id,
            msg.expense.total,
            msg.expense.date
        );

        let regional_msg = RegionalAdminMessage::PumpExpense {
            pump_address: msg.pump_address.clone(),
            expense: msg.expense.clone(),
            service_station_id: msg.service_station_id.clone(),
        };

        if let Some(company_conn_actor) = &self.company_connection {
            company_conn_actor.do_send(SendToClient::new(
                MsgType::PumpExpense,
                regional_msg.clone(),
            ));
            fuel_debug!(
                "✅ RegionalAdmin {} forwarded expense from pump '{}' (service_station: {}) to Company Leader",
                self.id,
                msg.pump_address,
                msg.service_station_id
            );
        } else {
            crash_info!(
                "❌ RegionalAdmin {} no connection found for Company Leader, buffering expense from pump '{}' (service_station: {})",
                self.id,
                msg.pump_address,
                msg.service_station_id
            );
            self.pending_company_messages.push(regional_msg); // Buffer the message
        }
    }
}

impl Handler<SendToCompanyFailed> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: SendToCompanyFailed, _ctx: &mut Self::Context) -> Self::Result {
        crash_info!(
            "❌ RegionalAdmin {} failed to send message to Company Leader, buffering message",
            self.id
        );
        self.pending_company_messages.push(msg.message.clone()); // Buffer the failed message
    }
}

impl Handler<CompanyPaymentProcessed> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: CompanyPaymentProcessed, _ctx: &mut Self::Context) -> Self::Result {
        let service_station_id = Some(msg.service_station_id.clone());

        if msg.accepted {
            fuel_info!(
                "✅ RegionalAdmin {} payment ACCEPTED for pump '{}' (service_station: {:?}): company_id={}, card_id={}, total=${}",
                self.id,
                msg.pump_address,
                service_station_id,
                msg.expense.company_id,
                msg.expense.card_id,
                msg.expense.total
            );
        } else {
            crash_info!(
                "❌ RegionalAdmin {} payment REJECTED for pump '{}' (service_station: {:?}): company_id={}, card_id={}, total=${}",
                self.id,
                msg.pump_address,
                service_station_id,
                msg.expense.company_id,
                msg.expense.card_id,
                msg.expense.total
            );
        }

        if let Some(service_station_id) = &service_station_id {
            if let Some(service_station_conn) =
                self.service_station_connections.get(service_station_id)
            {
                fuel_debug!(
                    "📤 RegionalAdmin {} sending PaymentProcessed to service_station '{}' leader",
                    self.id,
                    service_station_id
                );
                service_station_conn.do_send(SendPaymentProcessedToPump {
                    pump_address: msg.pump_address.clone(),
                    expense: msg.expense.clone(),
                    accepted: msg.accepted,
                    service_station_id: service_station_id.clone(),
                });
            } else {
                fuel_info!(
                    "⚠️ RegionalAdmin {} service_station '{}' leader not connected - buffering PaymentProcessed",
                    self.id,
                    service_station_id
                );

                // Check if this message is already buffered to prevent duplicates
                let already_buffered = self.pending_messages
                    .get(service_station_id)
                    .map(|msgs| {
                        msgs.iter().any(|pm| {
                            matches!(&pm.message, PendingMessageType::PaymentProcessed { pump_address, expense, accepted }
                                if pump_address == &msg.pump_address
                                && expense.card_id == msg.expense.card_id
                                && expense.total == msg.expense.total
                                && expense.date == msg.expense.date
                                && *accepted == msg.accepted)
                        })
                    })
                    .unwrap_or(false);

                if !already_buffered {
                    let pending_msg = PendingMessage {
                        msg_type: MsgType::PaymentProcessed,
                        message: PendingMessageType::PaymentProcessed {
                            pump_address: msg.pump_address.clone(),
                            expense: msg.expense.clone(),
                            accepted: msg.accepted,
                        },
                        timestamp: Self::current_timestamp(),
                    };

                    self.buffer_msg(service_station_id, pending_msg);
                }
            }
        } else {
            crash_info!(
                "❌ RegionalAdmin {} received PaymentProcessed with no service_station_id for pump '{}'",
                self.id,
                msg.pump_address
            );
        }
    }
}

impl Handler<SendToPumpSuccess> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: SendToPumpSuccess, _ctx: &mut Self::Context) -> Self::Result {
        fuel_debug!(
            "📤 RegionalAdmin {} confirmed successful delivery to pump '{}' for PaymentProcessed message",
            self.id,
            msg.pump_address
        );
        self.send_ack_to_company(&msg.pump_address, &msg.expense, MsgType::PaymentProcessed);
    }
}

impl Handler<SendToPumpFailed> for RegionalAdmin {
    type Result = ();

    fn handle(&mut self, msg: SendToPumpFailed, _ctx: &mut Self::Context) -> Self::Result {
        crash_info!(
            "❌ RegionalAdmin {} send FAILED for service_station '{}', msg_type {:?} - connection broken",
            self.id,
            msg.service_station_id,
            msg.msg_type
        );

        // Check if this message is already buffered to prevent duplicates
        let already_buffered = self
            .pending_messages
            .get(&msg.service_station_id)
            .map(|msgs| {
                msgs.iter().any(|pm| {
                    matches!(&pm.message, PendingMessageType::PaymentProcessed { pump_address, expense, accepted }
                        if pump_address == &msg.pump_address
                        && expense.card_id == msg.expense.card_id
                        && expense.total == msg.expense.total
                        && expense.date == msg.expense.date
                        && *accepted == msg.accepted)
                })
            })
            .unwrap_or(false);

        if !already_buffered {
            fuel_info!(
                "📦 RegionalAdmin {} buffering failed message for service_station '{}' (will retry when leader reconnects)",
                self.id,
                msg.service_station_id
            );
            let pending_msg = PendingMessage {
                msg_type: msg.msg_type,
                message: PendingMessageType::PaymentProcessed {
                    pump_address: msg.pump_address,
                    expense: msg.expense.clone(),
                    accepted: msg.accepted,
                },
                timestamp: Self::current_timestamp(),
            };

            self.buffer_msg(&msg.service_station_id, pending_msg);
        }
    }
}
