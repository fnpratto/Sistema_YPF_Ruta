use crate::message::PumpMessage;
use crate::message::SendToRegionalAdmin;
use crate::message::{
    ConnectToRegionalAdmin, Coordinator, Election, ExpenseConfirmed, ExpenseWithOrigin,
    GetLeaderInfo, PaymentProcessedResponse, SetLeader, StoreRegionalAdminConnection,
    TcpFuelRequest, TryNextRegionalAdmin, UpdatePumps,
};

use actix::{Actor, Addr, Context, Handler, ResponseFuture};
use common::constants::PRICEPERLITER;
use common::leader_election::{LeaderElection, LeaderEvent};
use common::message::Expense;
use common::message::{
    FuelRequestData, GetPendingCardResult, RemovePendingCardResult, StorePendingCardResult,
};

use common::message::RegionalAdminMessage;
use common::message::{FuelRequest, MessagesPump};
use common::message::{GasPumpYPFMessage, MsgType};
use common::message::{RefuelRequest, SetAddress, Start};
use common::tcp_protocol::TCPProtocol;
use common::{
    crash_info, election_debug, election_info, fuel_debug, fuel_info, leader_debug, leader_info,
    regional_admin_debug, regional_admin_error, regional_admin_info,
};
use log::{error, info};
use quinn::{Endpoint, RecvStream, ServerConfig};
use rand::Rng;
use rustls::{Certificate, PrivateKey};

use actix::AsyncContext;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_util::sync::CancellationToken;

#[allow(dead_code)]
pub struct Pump {
    pub address: String,
    pub pumps: HashMap<String, bool>, // puerto, activo
    pub addr_pumps: HashMap<u32, Addr<Pump>>,
    pub request_mailbox: VecDeque<RefuelRequest>,
    pub mailbox_expenses_lider: VecDeque<Expense>,
    pub buffered_expenses_pump: VecDeque<Expense>,
    pub is_leader: bool,
    pub leader_assigned: bool,
    pub service_station_address: Option<Addr<Pump>>,
    pub leader_addr: Option<Addr<Pump>>,
    pub leader_port: Option<u32>,
    pub listener_started: bool,
    pub election: Option<LeaderElection>,
    pub last_election: Option<Instant>,
    pub self_addr: Option<Addr<Pump>>,
    pub regional_admin_addrs: Vec<String>,
    pub current_regional_admin_index: usize,
    pub regional_admin_connection: Option<Addr<PumpRegionalAdminConnection>>,
    pub pending_card_results: HashMap<u32, (GasPumpYPFMessage, String)>,
    pub card_response_channels: HashMap<u32, tokio::sync::oneshot::Sender<bool>>,
    pub service_station_id: u32,
}

impl Actor for Pump {
    type Context = Context<Self>;
}

impl Pump {
    pub fn new(
        pumps: HashMap<String, bool>,
        _leader_assigned: bool,
        _leader_addr: Option<Addr<Pump>>,
        regional_admin_addrs: Vec<String>,
        _service_station_id: u32,
    ) -> Self {
        Self {
            address: String::new(),
            pumps,
            request_mailbox: VecDeque::new(),
            mailbox_expenses_lider: VecDeque::new(),
            buffered_expenses_pump: VecDeque::new(),
            is_leader: false,
            leader_assigned: false,
            service_station_address: None,
            leader_addr: None,
            self_addr: None,
            addr_pumps: HashMap::new(),
            leader_port: None,
            listener_started: false,
            election: None,
            last_election: None,
            regional_admin_addrs,
            current_regional_admin_index: 0,
            regional_admin_connection: None,
            pending_card_results: HashMap::new(),
            card_response_channels: HashMap::new(),
            service_station_id: _service_station_id,
        }
    }

    fn should_ignore_election(&self, msg: &Election) -> bool {
        let my_port = self
            .address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);

        // Ignore if the sender port is greater than or equal to my port
        msg.sender_port >= my_port
    }

    pub fn get_port(&self) -> u32 {
        self.address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0)
    }

    pub fn start_quic_listener(&self, pump_addr: Addr<Pump>) {
        let address = self.address.clone();

        tokio::spawn(async move {
            let server_config = match Pump::configure_server() {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("QUIC server config failed: {e}");
                    return;
                }
            };

            if let Err(e) = Pump::run_quic_listener(address, pump_addr, server_config).await {
                error!("QUIC listener failed: {}", e);
            }
        });
    }

    pub fn configure_server() -> anyhow::Result<quinn::ServerConfig> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let key = PrivateKey(cert.serialize_private_key_der());
        let cert = Certificate(cert.serialize_der()?);

        let server_config = ServerConfig::with_single_cert(vec![cert], key)?;

        Ok(server_config)
    }

    async fn run_quic_listener(
        address: String,
        pump_addr: Addr<Pump>,
        server_config: quinn::ServerConfig,
    ) -> anyhow::Result<()> {
        fuel_info!("Starting QUIC server on {}", address);

        // Create endpoint with the provided server config (don't call configure_server again!)
        let endpoint = Endpoint::server(server_config, address.parse()?)?;

        fuel_info!(" QUIC server successfully bound to {}", address);

        // Accept incoming connections
        while let Some(connecting) = endpoint.accept().await {
            let pump = pump_addr.clone();

            tokio::spawn(async move {
                match connecting.await {
                    Ok(connection) => {
                        fuel_debug!(
                            " QUIC connection established from {}",
                            connection.remote_address()
                        );

                        while let Ok(stream) = connection.accept_uni().await {
                            tokio::spawn(Pump::handle_quic_stream(stream, pump.clone()));
                        }
                    }
                    Err(e) => {
                        error!(" QUIC connection failed: {}", e);
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_quic_stream(mut stream: RecvStream, pump_addr: Addr<Pump>) {
        let mut buf = vec![0u8; 64 * 1024]; // Pre-allocate buffer

        match stream.read(&mut buf).await {
            Ok(Some(bytes_read)) => {
                buf.truncate(bytes_read);
            }
            Ok(None) => {
                error!("QUIC stream ended unexpectedly");
                return;
            }
            Err(e) => {
                error!("QUIC read error: {}", e);
                return;
            }
        }

        let req_str = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(_) => {
                error!("Invalid UTF-8 from QUIC");
                return;
            }
        };

        // Reuse your existing parsing logic
        match serde_json::from_str::<MessagesPump>(&req_str) {
            Ok(MessagesPump::RefuelRequest {
                request,
                card_address,
                ..
            }) => {
                let _ = pump_addr
                    .send(crate::message::TcpFuelRequest {
                        request: FuelRequestData {
                            company_id: request.company_id,
                            card_id: request.card_id,
                            liters: request.liters,
                        },
                        card_address,
                    })
                    .await;
            }
            Ok(_) => fuel_debug!("Unknown QUIC message"),
            Err(e) => error!("Failed to parse QUIC message: {}", e),
        }
    }

    fn ensure_election_initialized(&mut self) -> bool {
        if self.election.is_none() {
            election_info!("Force initializing election for pump {}", self.address);
            self.initialize_election_channel();
        }
        let is_ready = self.election.is_some();
        if !is_ready {
            election_info!("Election initialization failed for pump {}", self.address);
        }
        is_ready
    }

    /// Checks if the last election was too recent to trigger another.
    fn is_election_too_recent(&self) -> bool {
        if let Some(last_election) = self.last_election {
            last_election.elapsed() < Duration::from_secs(5) // Example debounce time: 5 seconds
        } else {
            false
        }
    }

    /// Processes the election message using the Bully algorithm.
    fn process_bully_election(&mut self, sender_port: u32) {
        election_info!(
            "Pump {} processing Bully election from {}",
            self.address,
            sender_port
        );

        let my_port = self
            .address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);

        if sender_port < my_port {
            election_info!(
                "Pump {} responding to election from {}",
                self.address,
                sender_port
            );
            if let Some(election) = &self.election {
                election.trigger_election();
            }
        } else {
            election_info!(
                "Pump {} deferring to higher port {}",
                self.address,
                sender_port
            );
        }
    }

    pub async fn spawn_with_setup(
        leader_assigned: bool,
        leader_handle: Option<Addr<Pump>>,
        bind_addr: String,
        regional_admin_addrs: Vec<String>,
        service_station_id: u32,
    ) -> Addr<Pump> {
        let actor_addr = Pump::create(move |_ctx| {
            let mut pump = Pump::new(
                HashMap::new(),
                leader_assigned,
                leader_handle,
                regional_admin_addrs.clone(),
                service_station_id,
            );

            pump.address = bind_addr.clone();
            fuel_debug!("Created pump instance with address {}", bind_addr);

            pump
        });

        actor_addr.do_send(crate::message::SetSelfAddr {
            addr: actor_addr.clone(),
        });

        actor_addr
    }

    pub fn listen_for_card_connection(&mut self, pump_addr: Addr<Pump>) {
        if self.listener_started {
            return;
        }
        self.listener_started = true;
        let addr = self.address.clone();

        // Start TCP listener for cards
        self.start_tcp_listener(&addr, pump_addr.clone());

        // Start QUIC listener for pump-to-pump
        self.start_quic_listener_for_pumps(pump_addr.clone());

        // Start random crash system
        self.start_random_crash_system();
    }

    pub fn start_tcp_listener(&self, address: &str, pump_addr: Addr<Pump>) {
        let address = address.to_string();
        tokio::spawn(async move {
            Self::run_tcp_listener(address, pump_addr).await;
        });
    }

    async fn run_tcp_listener(address: String, pump_addr: Addr<Pump>) {
        let listener = match TcpListener::bind(&address).await {
            Ok(l) => {
                fuel_info!("TCP server (for cards) bound to {}", address);
                l
            }
            Err(e) => {
                error!("Failed to bind TCP listener to address {}: {}", address, e);
                return;
            }
        };

        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    fuel_debug!("Card TCP connection from {}", peer_addr);
                    let peer_addr = peer_addr.to_string();
                    let pump_addr = pump_addr.clone();
                    tokio::spawn(async move {
                        Self::handle_tcp_stream(stream, peer_addr, pump_addr).await;
                    });
                }
                Err(e) => {
                    error!("Failed to accept TCP connection: {}", e);
                    continue;
                }
            }
        }
    }

    async fn handle_tcp_stream(
        stream: tokio::net::TcpStream,
        peer_addr: String,
        pump_addr: Addr<Pump>,
    ) {
        let (mut reader, mut writer) = stream.into_split();

        // First, read the message type to determine how to handle the connection
        let msg_type = match TCPProtocol::receive_msg_type_from_reader(&mut reader).await {
            Ok(mt) => mt,
            Err(_) => {
                // error!("Failed to read msg type from {}: {}", peer_addr, e);
                return;
            }
        };

        fuel_debug!("Received message type from {}: {:?}", peer_addr, msg_type);

        match msg_type {
            MsgType::GetPendingResult => {
                fuel_info!("Handling GetPendingResult from {}", peer_addr);
                Self::handle_get_pending_result(&mut reader, &mut writer, pump_addr).await;
            }
            MsgType::FuelRequest => {
                fuel_info!("Handling FuelRequest from {}", peer_addr);
                Self::handle_fuel_request(&mut reader, &mut writer, peer_addr, pump_addr).await;
            }
            _ => {
                error!(
                    "Unexpected MsgType received from {}: {:?}",
                    peer_addr, msg_type
                );
                // Send error response
                let error_response = GasPumpYPFMessage::FuelComplete { success: false };
                let _ = TCPProtocol::send_with_writer(
                    &mut writer,
                    MsgType::FuelComplete,
                    &error_response,
                )
                .await;
            }
        }
    }

    async fn handle_get_pending_result(
        reader: &mut OwnedReadHalf,
        writer: &mut OwnedWriteHalf,
        pump_addr: Addr<Pump>,
    ) {
        fuel_info!("Processing GetPendingResult request");

        let query = match TCPProtocol::receive_msg_from_reader::<GetPendingCardResult>(reader).await
        {
            Ok(q) => q,
            Err(e) => {
                error!("Failed to read GetPendingCardResult payload: {}", e);
                // Send error response
                let error_response = GasPumpYPFMessage::FuelComplete { success: false };
                let _ =
                    TCPProtocol::send_with_writer(writer, MsgType::FuelComplete, &error_response)
                        .await;
                return;
            }
        };

        fuel_debug!("Checking for pending result for card {}", query.card_id);

        let pending_result = pump_addr
            .send(GetPendingCardResult {
                card_id: query.card_id,
            })
            .await
            .ok()
            .flatten();

        if let Some(result) = pending_result {
            fuel_info!(
                "Found pending result for card {}, sending response",
                query.card_id
            );
            let _ = TCPProtocol::send_with_writer(writer, MsgType::FuelComplete, &result).await;

            let _ = pump_addr
                .send(RemovePendingCardResult {
                    card_id: query.card_id,
                })
                .await;
        } else {
            fuel_debug!("No pending result found for card {}", query.card_id);
            // Send a "no pending result" response
            let empty_response = GasPumpYPFMessage::FuelComplete { success: false };
            let _ =
                TCPProtocol::send_with_writer(writer, MsgType::NoPendingResult, &empty_response)
                    .await;
        }

        fuel_info!(
            "Completed GetPendingResult processing for card {}",
            query.card_id
        );
    }

    async fn handle_fuel_request(
        reader: &mut OwnedReadHalf,
        writer: &mut OwnedWriteHalf,
        peer_addr: String,
        pump_addr: Addr<Pump>,
    ) {
        fuel_info!("Processing FuelRequest from {}", peer_addr);

        // Parse the fuel request payload
        let (req_data, card_addr) = match Self::parse_fuel_request_payload(reader).await {
            Ok(v) => v,
            Err(e) => {
                error!("Parse error from {}: {}", peer_addr, e);
                // Send immediate error response
                let error_response = GasPumpYPFMessage::FuelComplete { success: false };
                let _ =
                    TCPProtocol::send_with_writer(writer, MsgType::FuelComplete, &error_response)
                        .await;
                return;
            }
        };

        fuel_info!(
            "Received fuel request from card {} at address {} - {} liters",
            req_data.card_id,
            card_addr,
            req_data.liters
        );

        // Check if there are pending results for this card ID BEFORE processing the new request
        let pending_result = pump_addr
            .send(GetPendingCardResult {
                card_id: req_data.card_id,
            })
            .await;

        // If there's a pending result, send it first and remove it
        if let Ok(Some(pending_msg)) = pending_result {
            fuel_info!(
                "Sending pending result to reconnected card {} at address {}",
                req_data.card_id,
                card_addr
            );

            let _ =
                TCPProtocol::send_with_writer(writer, MsgType::FuelComplete, &pending_msg).await;

            let _ = pump_addr
                .send(RemovePendingCardResult {
                    card_id: req_data.card_id,
                })
                .await;

            // After sending pending result, close this connection since the card got its pending result
            fuel_info!(
                "Sent pending result and closing connection for card {}",
                req_data.card_id
            );
            return;
        }

        // Process the new fuel request
        fuel_info!("Processing new fuel request for card {}", req_data.card_id);

        let ack = match pump_addr
            .send(crate::message::TcpFuelRequest {
                request: req_data.clone(),
                card_address: card_addr.clone(),
            })
            .await
        {
            Ok(ack) => {
                fuel_info!(
                    "Pump actor responded with ack: {} for card {}",
                    ack,
                    req_data.card_id
                );
                ack
            }
            Err(e) => {
                error!("Failed to send TcpFuelRequest to pump actor: {}", e);
                false
            }
        };

        // If the expense was successfully sent to the leader, keep connection open
        // and wait for PaymentProcessedResponse
        if ack {
            fuel_info!(
                "Expense sent to leader for card {} - keeping connection open to wait for payment confirmation",
                req_data.card_id
            );

            let (tx, rx) = tokio::sync::oneshot::channel();

            let _ = pump_addr
                .send(StoreCardResponseChannel {
                    card_id: req_data.card_id,
                    sender: tx,
                })
                .await;

            match tokio::time::timeout(Duration::from_secs(30), rx).await {
                Ok(Ok(payment_accepted)) => {
                    fuel_info!(
                        "Received payment confirmation for card {}: {}",
                        req_data.card_id,
                        payment_accepted
                    );

                    if let Err(e) = Self::send_fuel_complete(writer, payment_accepted).await {
                        error!(
                            "Failed to send FuelComplete to card {}: {}",
                            req_data.card_id, e
                        );

                        let pending_response = GasPumpYPFMessage::FuelComplete {
                            success: payment_accepted,
                        };
                        let _ = pump_addr
                            .send(StorePendingCardResult {
                                card_id: req_data.card_id,
                                card_address: card_addr.clone(),
                                result: pending_response,
                            })
                            .await;
                        fuel_info!(
                            "Stored pending result for card {} due to send failure",
                            req_data.card_id
                        );
                    } else {
                        fuel_info!(
                            "✅ Successfully sent FuelComplete to card {}",
                            req_data.card_id
                        );
                    }
                }
                Ok(Err(_)) => {
                    error!(
                        "Payment response channel closed for card {}",
                        req_data.card_id
                    );
                    let _ = Self::send_fuel_complete(writer, false).await;
                }
                Err(_) => {
                    error!(
                        "Timeout waiting for payment response for card {}",
                        req_data.card_id
                    );
                    let _ = Self::send_fuel_complete(writer, false).await;
                }
            }
        } else {
            fuel_info!(
                "Failed to send expense to leader for card {} - sending immediate failure",
                req_data.card_id
            );

            if let Err(e) = Self::send_fuel_complete(writer, false).await {
                error!(
                    "Failed to send failure response to card {}: {}",
                    req_data.card_id, e
                );
            } else {
                fuel_info!("Sent failure response to card {}", req_data.card_id);
            }
        }
    }

    async fn parse_fuel_request_payload(
        reader: &mut OwnedReadHalf,
    ) -> Result<(FuelRequestData, String), String> {
        let msg = TCPProtocol::receive_msg_from_reader::<GasPumpYPFMessage>(reader)
            .await
            .map_err(|e| format!("failed to read FuelRequest payload: {}", e))?;

        if let GasPumpYPFMessage::FuelRequest {
            request,
            card_address,
        } = msg
        {
            Ok((request, card_address))
        } else {
            Err("unexpected GasPumpYPFMessage variant".to_string())
        }
    }

    pub async fn parse_incoming_fuel_request_from_tcp(
        reader: &mut tokio::net::tcp::OwnedReadHalf,
    ) -> Result<(FuelRequestData, String), String> {
        let msg_type = TCPProtocol::receive_msg_type_from_reader(reader)
            .await
            .map_err(|e| format!("failed to read msg type: {}", e))?;
        match msg_type {
            MsgType::FuelRequest => Self::parse_fuel_request_payload(reader).await,
            _ => Err(format!("unexpected MsgType: {:?}", msg_type)),
        }
    }

    pub async fn process_fuel_request(
        request: FuelRequestData,
        originating_pump_address: String,
        leader: Option<Addr<Pump>>,
        leader_port: Option<u32>,
        election: Option<&LeaderElection>,
    ) -> Result<bool, String> {
        fuel_info!("Processing fuel request: {} liters", request.liters);
        if request.liters == 0 {
            return Err("liters must be > 0".to_string());
        }

        let expense = common::message::Expense {
            company_id: request.company_id,
            card_id: request.card_id,
            total: request.liters * PRICEPERLITER,
            date: "2025-11-21".to_string(),
        };

        if let Some(leader_addr) = leader {
            fuel_info!(" Sending expense to leader: {}", leader_port.unwrap_or(0));

            match leader_addr
                .send(crate::message::ExpenseWithOrigin {
                    expense: expense.clone(),
                    originating_pump: originating_pump_address,
                })
                .await
            {
                Ok(_) => {
                    fuel_info!(" Expense sent successfully to leader");
                    Ok(true)
                }
                Err(e) => {
                    error!(" Failed to send expense to leader: {}", e);
                    fuel_info!("Leader appears to be down - triggering election");

                    if let Some(election) = election {
                        election.trigger_election();
                    }
                    Err("leader communication failed".to_string())
                }
            }
        } else {
            Err("no leader address".to_string())
        }
    }

    pub async fn handle_connection(
        &self,
        _socket: &UdpSocket,
        buffer: &[u8],
        peer_addr: &str,
        service_station_address: Option<Addr<Pump>>,
    ) -> Result<(), String> {
        if let Ok(Some((req, _card_addr_str))) = Self::receive_fuelrequest(buffer, peer_addr).await
        {
            let proto = FuelRequestData {
                company_id: req.company_id,
                card_id: req.card_id,
                liters: req.liters,
            };

            let leader_opt = service_station_address;
            match Self::process_fuel_request(
                proto,
                self.address.clone(),
                leader_opt,
                self.leader_port,
                self.election.as_ref(),
            )
            .await
            {
                Ok(ack) => {
                    if ack {
                        fuel_info!("leader acknowledged UDP expense");
                    } else {
                        fuel_info!("leader rejected UDP expense");
                    }
                }
                Err(e) => error!("failed to process UDP request: {}", e),
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    async fn send_message_to_lider(pump: Addr<Pump>, msg: Expense) -> Result<bool, String> {
        match pump.send(msg).await {
            Ok(ack) => Ok(ack),
            Err(e) => Err(format!("Failed to send message to pump: {}", e)),
        }
    }

    pub async fn receive_fuelrequest(
        buffer: &[u8],
        peer_addr: &str,
    ) -> Result<Option<(FuelRequest, String)>, String> {
        let req_str = String::from_utf8_lossy(buffer);
        fuel_debug!("Raw message received from {}: {}", peer_addr, req_str);

        match serde_json::from_str::<MessagesPump>(&req_str) {
            Ok(MessagesPump::RefuelRequest {
                op_code: _,
                request,
                card_address,
            }) => {
                let fuel_request = FuelRequest {
                    company_id: request.company_id,
                    card_id: request.card_id,
                    liters: request.liters,
                };
                fuel_info!(
                    "Processing fuel request - Company: {}, Card: {}, Liters: {}",
                    fuel_request.company_id,
                    fuel_request.card_id,
                    fuel_request.liters
                );
                Ok(Some((fuel_request, card_address)))
            }
            Ok(_) => Ok(None),
            Err(e) => Err(format!("Failed to parse request: {}", e)),
        }
    }

    pub async fn send_fuel_complete(writer: &mut OwnedWriteHalf, ack: bool) -> Result<(), String> {
        let complete = GasPumpYPFMessage::FuelComplete { success: ack };
        TCPProtocol::send_with_writer(writer, MsgType::FuelComplete, &complete)
            .await
            .map_err(|e| format!("tcp send error: {}", e))
    }

    pub fn initialize_election_channel(&mut self) {
        election_debug!("Initializing election channel for pump {}", self.address);

        // Only initialize if election doesn't exist
        if self.election.is_some() {
            election_debug!("Election already exists for pump {}", self.address);
            return;
        }

        // Validate we have the necessary data
        if self.address.is_empty() {
            election_info!("Cannot initialize election - address not set");
            return;
        }

        if self.pumps.is_empty() {
            election_info!("Cannot initialize election - no pumps configured");
            return;
        }

        // Use only tokio channels
        let (tx, mut rx) = tokio_mpsc::channel::<LeaderEvent>(100);

        // Initialize LeaderElection with the tokio sender
        match LeaderElection::new(&self.address, self.pumps.clone(), Some(tx)) {
            Ok(election) => {
                self.election = Some(election);
                election_info!("Election system initialized for pump {}", self.address);
            }
            Err(e) => {
                error!(
                    "Failed to initialize LeaderElection for pump {}: {}",
                    self.address, e
                );
                return;
            }
        }

        // Clone addrs for the background task
        let addrs = self.addr_pumps.clone();
        let pump_address = self.address.clone();

        tokio::spawn(async move {
            election_debug!("Starting election event listener for pump {}", pump_address);

            while let Some(evt) = rx.recv().await {
                match evt {
                    LeaderEvent::Coordinator(id) => {
                        leader_info!(
                            "Pump {} received coordinator event for leader {}",
                            pump_address,
                            id
                        );
                        for (_port, addr) in addrs.iter() {
                            addr.do_send(SetLeader {
                                leader_port: id as u32,
                            });
                        }
                    }
                    LeaderEvent::Election(id) => {
                        election_info!("Pump {} received election event from {}", pump_address, id);
                        for (_port, addr) in addrs.iter() {
                            addr.do_send(Election {
                                sender_port: id as u32,
                            });
                        }
                    }
                }
            }
            election_debug!("Election event listener ended for pump {}", pump_address);
        });
    }

    /// Enhanced crash simulation with recovery
    fn simulate_crash(&mut self) {
        crash_info!("PUMP {} SIMULATING CRASH - resetting state", self.address);

        if self.is_leader {
            self.broadcast_leader_failure();
        }

        // Reset state
        self.is_leader = false;
        self.leader_assigned = false;
        self.leader_port = None;
        self.leader_addr = None;

        if let Some(election) = &self.election {
            election.stop();
        }
        self.election = None;

        let lost_expenses = self.buffered_expenses_pump.len();
        if lost_expenses > 0 {
            crash_info!("Lost {} pending expenses in crash", lost_expenses);
            self.buffered_expenses_pump.clear();
        }

        self.schedule_crash_recovery();
    }

    /// Schedule recovery from cras
    fn schedule_crash_recovery(&mut self) {
        if let Some(self_addr) = self.self_addr.clone() {
            // Fixed 3-second recovery delay
            let recovery_delay = 3000;

            crash_info!("Scheduling recovery in {}ms", recovery_delay);

            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(recovery_delay)).await;
                let _ = self_addr.send(CrashRecovery).await;
            });
        }
    }

    pub fn connect_to_regional_admin(&mut self) {
        if !self.is_leader {
            regional_admin_info!(
                "Pump {} is NOT a leader - skipping Regional Admin connection",
                self.address
            );
            return;
        }

        if self.regional_admin_connection.is_some() {
            return;
        }

        if self.regional_admin_addrs.is_empty() {
            error!("No regional admin addresses configured");
            return;
        }

        let current_addr = self.regional_admin_addrs[self.current_regional_admin_index].clone();

        let pump_addr = match &self.self_addr {
            Some(addr) => addr.clone(),
            None => {
                error!("Cannot connect to Regional Admin: self_addr not set");
                return;
            }
        };

        let my_address = self.address.clone();
        let service_station_id = self.service_station_id;
        let total_admins = self.regional_admin_addrs.len();
        let current_index = self.current_regional_admin_index;

        let province_info = self.get_province_info_for_address(&current_addr);

        regional_admin_info!(
            "LEADER Pump {} (service_station: {}) attempting connection to Regional Admin for {} (attempt {}/{})",
            my_address,
            service_station_id,
            province_info,
            current_index + 1,
            total_admins
        );

        let regional_admin_addrs_clone = self.regional_admin_addrs.clone();
        let pump_addr_for_connection = pump_addr.clone();

        actix::spawn(async move {
            match tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::TcpStream::connect(&current_addr),
            )
            .await
            {
                Ok(Ok(mut stream)) => {
                    regional_admin_info!(
                        "✅ LEADER Pump {} (service_station: {}) SUCCESSFULLY CONNECTED to Regional Admin for {}",
                        my_address,
                        service_station_id,
                        province_info
                    );

                    let handshake = format!("PUMP {} {}\n", my_address, service_station_id);
                    if let Err(e) = stream.write_all(handshake.as_bytes()).await {
                        regional_admin_error!(
                            "❌ Pump {} failed to send handshake to Regional Admin: {}",
                            my_address,
                            e
                        );
                        return;
                    }
                    if let Err(e) = stream.flush().await {
                        regional_admin_error!(
                            "❌ Pump {} failed to flush handshake to Regional Admin: {}",
                            my_address,
                            e
                        );
                        return;
                    }

                    let (reader, writer) = stream.into_split();
                    let conn_actor = PumpRegionalAdminConnection::create(|_ctx| {
                        let mut conn =
                            PumpRegionalAdminConnection::new(writer, pump_addr_for_connection);
                        let cancel_token = CancellationToken::new();
                        conn.reader_task_cancel_token = Some(cancel_token.clone());
                        conn.spawn_reader_task(reader, cancel_token);
                        conn
                    });

                    pump_addr.do_send(StoreRegionalAdminConnection {
                        connection: conn_actor,
                    });
                }
                Ok(Err(e)) => {
                    regional_admin_error!(
                        "❌ FAILED to connect to Regional Admin for {} (nearest admin): Connection error - {}",
                        province_info,
                        e
                    );
                    regional_admin_info!(
                        "🔄 Pump {} initiating FAILOVER due to connection failure to nearest Regional Admin ({})",
                        my_address,
                        province_info
                    );

                    // Try failover to next regional admin
                    pump_addr.do_send(TryNextRegionalAdmin {
                        failed_address: current_addr,
                        regional_admin_addrs: regional_admin_addrs_clone,
                        current_index,
                    });
                }
                Err(_) => {
                    regional_admin_error!(
                        "⏰ TIMEOUT connecting to Regional Admin for {} (nearest admin): No response received within 5 seconds",
                        province_info
                    );
                    regional_admin_info!(
                        "🔄 Pump {} initiating FAILOVER due to NO RESPONSE from nearest Regional Admin ({})",
                        my_address,
                        province_info
                    );

                    // Try failover to next regional admin due to timeout
                    pump_addr.do_send(TryNextRegionalAdmin {
                        failed_address: current_addr,
                        regional_admin_addrs: regional_admin_addrs_clone,
                        current_index,
                    });
                }
            }
        });
    }

    fn try_next_regional_admin(&mut self, failed_address: String, current_index: usize) {
        if self.regional_admin_addrs.is_empty() {
            regional_admin_error!("No regional admin addresses available for failover");
            return;
        }

        // Move to next regional admin in the list
        let next_index = (current_index + 1) % self.regional_admin_addrs.len();

        // If we've gone through all admins, wait before retrying from the beginning
        if next_index == 0 && current_index > 0 {
            regional_admin_error!(
                "🔄 Pump {} completed FULL SEQUENTIAL FAILOVER CYCLE - all {} regional admins failed to respond",
                self.address,
                self.regional_admin_addrs.len()
            );
            regional_admin_info!(
                "⏳ Pump {} waiting 10 seconds before retrying connection cycle to regional admins",
                self.address
            );

            self.current_regional_admin_index = 0;
            self.regional_admin_connection = None;

            // Schedule retry with proper error handling
            if let Some(self_addr) = self.self_addr.clone() {
                let pump_address = self.address.clone();
                tokio::spawn(async move {
                    regional_admin_info!(
                        "Pump {} starting 10-second wait before Regional Admin retry...",
                        pump_address
                    );

                    tokio::time::sleep(Duration::from_secs(10)).await;

                    regional_admin_info!(
                        "Pump {} retrying Regional Admin connection after failover cycle timeout",
                        pump_address
                    );

                    if let Err(e) = self_addr.send(ConnectToRegionalAdmin).await {
                        regional_admin_error!(
                            "❌ Pump {} failed to send ConnectToRegionalAdmin retry message: {}",
                            pump_address,
                            e
                        );
                    } else {
                        regional_admin_info!(
                            "✅ Pump {} successfully sent ConnectToRegionalAdmin retry message",
                            pump_address
                        );
                    }
                });
            } else {
                regional_admin_error!(
                    "❌ Pump {} cannot retry - self_addr not available",
                    self.address
                );
            }
            return;
        }

        self.current_regional_admin_index = next_index;
        let next_addr = &self.regional_admin_addrs[next_index];

        let (failed_province_info, next_province_info) = (
            self.get_province_info_for_address(&failed_address),
            self.get_province_info_for_address(next_addr),
        );

        regional_admin_info!(
            "🔄 Pump {} SEQUENTIAL FAILOVER: {} (no response) → {} (attempting backup)",
            self.address,
            failed_province_info,
            next_province_info
        );
        regional_admin_info!(
            "📍 Failover progress for Pump {}: attempt {}/{} - trying backup regional admin",
            self.address,
            next_index + 1,
            self.regional_admin_addrs.len()
        );

        // Reset connection and try the next one
        self.regional_admin_connection = None;

        // Small delay before attempting next connection
        if let Some(self_addr) = self.self_addr.clone() {
            let pump_address = self.address.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;

                regional_admin_info!(
                    "🔄 Pump {} attempting next Regional Admin after brief delay",
                    pump_address
                );

                if let Err(e) = self_addr.send(ConnectToRegionalAdmin).await {
                    regional_admin_error!(
                        "❌ Pump {} failed to send next Regional Admin connection attempt: {}",
                        pump_address,
                        e
                    );
                } else {
                    regional_admin_debug!(
                        "✅ Pump {} sent next Regional Admin connection attempt",
                        pump_address
                    );
                }
            });
        }
    }

    fn get_province_info_for_address(&self, address: &str) -> String {
        common::constants::PROVINCES
            .iter()
            .find(|p| p.address == address)
            .map(|p| format!("{} (ID: {})", p.name, p.id))
            .unwrap_or_else(|| format!("Unknown ({})", address))
    }

    /// Get this pump's ID for comparison in Bully algorithm
    fn get_my_id(&self) -> usize {
        self.address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0) as usize
    }

    #[allow(dead_code)]
    fn start_expense_timeout(&mut self, ctx: &mut Context<Self>) {
        if !self.buffered_expenses_pump.is_empty() {
            ctx.run_later(Duration::from_secs(1), |act, _ctx| {
                if !act.buffered_expenses_pump.is_empty() {
                    fuel_info!("Expense timeout - clearing pending expense");
                    act.buffered_expenses_pump.clear();
                }
            });
        }
    }

    fn should_crash(&self) -> bool {
        let mut rng = rand::rng();
        rng.random_bool(0.03) // 3% general crash probability
    }

    /// Enhanced should_crash with different probabilities for different operations
    fn should_crash_for_operation(&self, operation: CrashableOperation) -> bool {
        let mut rng = rand::rng();
        let crash_probability = match operation {
            CrashableOperation::ElectionProcessing => 0.01, // 1% during elections
            CrashableOperation::ExpenseForwarding => 0.10,  // 10% during expense forwarding
        };

        rng.random_bool(crash_probability)
    }

    /// Crash during election processing
    fn crash_safe_election_processing(&mut self, msg: &Election) -> bool {
        if self.should_crash_for_operation(CrashableOperation::ElectionProcessing) {
            crash_info!("CRASH during election from {}", msg.sender_port);
            self.simulate_crash();
            false // Don't process election
        } else {
            true // Process election normally
        }
    }

    fn broadcast_leader_failure(&self) {
        crash_info!(
            "Broadcasting leader failure to {} pumps",
            self.addr_pumps.len()
        );

        for (port, addr) in self.addr_pumps.iter() {
            // Skip sending to self since we're about to stop
            if *port != self.get_port() {
                let election_msg = Election {
                    sender_port: 0, // Special value indicating leader failure
                };

                match addr.try_send(election_msg) {
                    Ok(()) => {
                        crash_info!("Sent leader failure notification to pump {}", port);
                    }
                    Err(e) => {
                        error!(
                            "Failed to send leader failure notification to pump {}: {}",
                            port, e
                        );
                    }
                }
            }
        }

        crash_info!("Leader failure broadcast complete");
    }

    /// Start QUIC listener specifically for pump-to-pump communication
    pub fn start_quic_listener_for_pumps(&self, pump_addr: Addr<Pump>) {
        let pump_quic_address = self.get_pump_quic_address();

        tokio::spawn(async move {
            let server_config = match Pump::configure_server() {
                Ok(cfg) => cfg,
                Err(e) => {
                    error!("QUIC server config failed: {e}");
                    return;
                }
            };

            if let Err(e) =
                Pump::run_pump_quic_listener(pump_quic_address, pump_addr, server_config).await
            {
                error!("Pump QUIC listener failed: {}", e);
            }
        });
    }

    /// Get QUIC address for pump-to-pump communication (different port)
    fn get_pump_quic_address(&self) -> String {
        let base_port = self
            .address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(10000);

        // Use +100 offset instead of +1000 to avoid conflicts
        format!("127.0.0.1:{}", base_port + 100)
    }

    /// QUIC listener specifically for pump-to-pump messages
    async fn run_pump_quic_listener(
        address: String,
        pump_addr: Addr<Pump>,
        server_config: quinn::ServerConfig,
    ) -> anyhow::Result<()> {
        election_info!("Starting PUMP QUIC server on {}", address);

        let endpoint = Endpoint::server(server_config, address.parse()?)?;
        election_info!("PUMP QUIC server bound to {}", address);

        while let Some(connecting) = endpoint.accept().await {
            let pump = pump_addr.clone();

            tokio::spawn(async move {
                match connecting.await {
                    Ok(connection) => {
                        election_debug!(
                            "QUIC pump connection from {}",
                            connection.remote_address()
                        );

                        // Handle pump-to-pump streams
                        while let Ok(stream) = connection.accept_uni().await {
                            tokio::spawn(Pump::handle_pump_quic_stream(stream, pump.clone()));
                        }
                    }
                    Err(e) => {
                        error!("PUMP QUIC connection failed: {}", e);
                    }
                }
            });
        }

        Ok(())
    }

    /// Handle QUIC streams for pump-to-pump communication
    async fn handle_pump_quic_stream(mut stream: RecvStream, pump_addr: Addr<Pump>) {
        let buf = Vec::with_capacity(4096); // Smaller buffer for pump messages

        if let Err(e) = stream.read_to_end(4096).await {
            error!("PUMP QUIC read error: {}", e);
            return;
        }

        let msg_str = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(_) => {
                error!("Invalid UTF-8 in pump QUIC message");
                return;
            }
        };

        election_debug!("PUMP QUIC message: {}", msg_str);

        // Parse pump-to-pump messages
        match Self::parse_pump_message(&msg_str) {
            Ok(pump_msg) => {
                Self::handle_pump_message(pump_msg, pump_addr).await;
            }
            Err(e) => {
                error!("Failed to parse pump QUIC message: {}", e);
            }
        }
    }

    /// Parse pump-to-pump messages from QUIC
    fn parse_pump_message(msg_str: &str) -> Result<PumpMessage, String> {
        serde_json::from_str::<PumpMessage>(msg_str).map_err(|e| format!("Parse error: {}", e))
    }

    /// Handle parsed pump-to-pump messages
    async fn handle_pump_message(msg: PumpMessage, pump_addr: Addr<Pump>) {
        match msg {
            PumpMessage::Election { sender_port } => {
                election_info!(" QUIC Election from pump {}", sender_port);
                let _ = pump_addr.send(Election { sender_port }).await;
            }
            PumpMessage::Coordinator { sender_port } => {
                leader_info!(" QUIC Coordinator from pump {}", sender_port);
                let _ = pump_addr.send(Coordinator { sender_port }).await;
            }
            PumpMessage::Expense(expense) => {
                fuel_info!("QUIC Expense: ${}", expense.total);
                let _ = pump_addr.send(expense).await;
            }
            PumpMessage::LeaderFailure {
                failed_leader_port,
                sender_port: 0,
            } => {
                crash_info!(" QUIC Leader failure notification: {}", failed_leader_port);
                let _ = pump_addr.send(Election { sender_port: 0 }).await;
            }
            PumpMessage::LeaderFailure { sender_port, .. } if sender_port >= 1 => {
                crash_info!(
                    " QUIC Leader failure notification from pump {}",
                    sender_port
                );
            }
            PumpMessage::LeaderFailure { sender_port, .. } => {
                crash_info!(
                    "QUIC Leader failure notification with unexpected sender_port: {}",
                    sender_port
                );
            }
        }
    }

    fn trigger_emergency_election(&mut self) {
        election_info!(
            "Triggering EMERGENCY election for pump {} - no leader available",
            self.address
        );

        // Reset leader state completely
        self.leader_assigned = false;
        self.is_leader = false;
        self.leader_port = None;
        self.leader_addr = None;

        // Disconnect from Regional Admin if connected
        if self.regional_admin_connection.is_some() {
            regional_admin_info!("Disconnecting from Regional Admin due to emergency election");
            self.regional_admin_connection = None;
        }

        // Force reinitialize election if needed
        if !self.ensure_election_initialized() {
            error!("Cannot trigger emergency election - initialization failed");
            return;
        }

        // Reset and trigger election
        if let Some(election) = &self.election {
            election.reset();

            // Add small delay to ensure reset is processed
            let election_clone = election.clone();
            let pump_address = self.address.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                election_info!("Pump {} triggering election after reset", pump_address);
                election_clone.trigger_election();
            });

            self.last_election = Some(Instant::now());
        }
    }

    /// Send message to another pump via QUIC
    pub async fn send_quic_message_to_pump(
        target_address: &str,
        message: PumpMessage,
    ) -> Result<(), String> {
        let base_port =
            Self::extract_port_from_address(target_address).ok_or("Invalid target address")?;

        // Use a smaller offset that won't conflict with other stations
        // Since each station has only 10 pumps (0-9), use +100 offset
        let target_quic_port = base_port + 100;

        let target_quic_addr = format!("127.0.0.1:{}", target_quic_port);

        election_debug!(
            "Sending QUIC message to pump {} (QUIC port: {})",
            target_address,
            target_quic_port
        );

        // ... rest of the function remains the same
        // Create client endpoint
        let bind_addr = "0.0.0.0:0"
            .parse()
            .map_err(|e| format!("Invalid bind address: {}", e))?;

        let mut endpoint = Endpoint::client(bind_addr)
            .map_err(|e| format!("Failed to create client endpoint: {}", e))?;
        let client_config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(std::sync::Arc::new(SkipServerVerification))
            .with_no_client_auth();

        endpoint.set_default_client_config(quinn::ClientConfig::new(std::sync::Arc::new(
            client_config,
        )));

        // Connect and send
        let target_socket_addr = target_quic_addr
            .parse()
            .map_err(|e| format!("Invalid target address {}: {}", target_quic_addr, e))?;

        let connection = endpoint
            .connect(target_socket_addr, "localhost")
            .map_err(|e| format!("Failed to connect: {}", e))?
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;

        let mut stream = connection
            .open_uni()
            .await
            .map_err(|e| format!("Failed to open stream: {}", e))?;

        let msg_bytes =
            serde_json::to_vec(&message).map_err(|e| format!("Serialization failed: {}", e))?;

        stream
            .write_all(&msg_bytes)
            .await
            .map_err(|e| format!("Write failed: {}", e))?;

        stream
            .finish()
            .await
            .map_err(|e| format!("Stream finish failed: {}", e))?;

        election_debug!(" QUIC message sent to pump {}", target_quic_addr);
        Ok(())
    }

    fn extract_port_from_address(address: &str) -> Option<u32> {
        address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
    }

    /// Background task that randomly triggers crashes
    pub fn start_random_crash_system(&self) {
        if let Some(self_addr) = self.self_addr.clone() {
            tokio::spawn(async move {
                loop {
                    // Wait 20 seconds between crash checks
                    tokio::time::sleep(Duration::from_secs(60)).await;

                    crash_info!(" Random crash timer triggered!");

                    // Always trigger crash every check
                    match self_addr.send(RandomCrashTrigger).await {
                        Ok(_) => {
                            break;
                        }
                        Err(e) => {
                            error!("Failed to send crash trigger: {}", e);
                        }
                    }
                }
            });
        }
    }
    fn recover_pending_expenses(&mut self) {
        let expense_count = self.buffered_expenses_pump.len();

        if expense_count == 0 {
            leader_info!("No pending expenses to recover");
            return;
        }

        leader_info!(" Starting recovery of {} pending expenses", expense_count);

        match &self.leader_addr {
            Some(leader_addr) => {
                let leader_addr = leader_addr.clone();
                let expenses: Vec<_> = self.buffered_expenses_pump.drain(..).collect();
                let my_address = self.address.clone();

                tokio::spawn(async move {
                    let mut successful_recoveries = 0;
                    let mut failed_recoveries = 0;

                    for expense in expenses {
                        leader_info!(
                            " Pump {} resending expense ${} to new leader",
                            my_address,
                            expense.total
                        );

                        match leader_addr.send(expense).await {
                            Ok(_) => {
                                successful_recoveries += 1;
                                leader_debug!(" Expense recovery successful");
                            }
                            Err(e) => {
                                failed_recoveries += 1;
                                error!(" Failed to recover expense: {}", e);
                            }
                        }
                    }

                    leader_info!(
                        " Expense recovery completed for pump {}: {} successful, {} failed",
                        my_address,
                        successful_recoveries,
                        failed_recoveries
                    );
                });
            }
            None => {
                error!(" Cannot recover expenses: no leader address available");
            }
        }
    }
}

// Skip certificate verification for self-signed certsVerification { for self-signed certs
struct SkipServerVerification;

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

impl Handler<UpdatePumps> for Pump {
    type Result = ();

    fn handle(&mut self, msg: UpdatePumps, _ctx: &mut Self::Context) -> Self::Result {
        fuel_debug!(
            "Pump {} received UpdatePumps with {} entries",
            self.address,
            msg.addr_pumps.len()
        );

        self.pumps.clear();
        self.addr_pumps = msg.addr_pumps;

        for (port, _addr) in self.addr_pumps.iter() {
            self.pumps.insert(format!("{}", port), true);
        }

        // Force reinitialize election with new member list
        if let Some(election) = &self.election {
            election.stop();
        }
        self.election = None;
        self.initialize_election_channel();

        if let Some(election) = &self.election {
            election.update_members_sync(self.address.clone(), self.pumps.clone());

            // Determine if we should trigger initial election
            let my_port = self
                .address
                .rsplit(':')
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(0);

            let max_port = self.addr_pumps.keys().max().copied().unwrap_or(0);

            // Add delay and trigger election from max port to avoid race conditions
            if !self.leader_assigned && my_port == max_port {
                let election_clone = election.clone();
                let pump_address = self.address.clone();

                tokio::spawn(async move {
                    // Wait a bit for all pumps to be ready
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    election_info!(
                        "Pump {} (max port) triggering initial election",
                        pump_address
                    );
                    election_clone.trigger_election();
                });
            }

            // Schedule backup election trigger in case the first one fails
            let self_addr = _ctx.address();
            let pump_address_backup = self.address.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                election_info!("Backup election trigger for pump {}", pump_address_backup);
                self_addr.do_send(Election { sender_port: 0 });
            });
        }
    }
}

impl Handler<SetLeader> for Pump {
    type Result = ();

    fn handle(&mut self, msg: SetLeader, _ctx: &mut Self::Context) -> Self::Result {
        if self.leader_port == Some(msg.leader_port) && self.leader_assigned {
            election_debug!("Duplicate SetLeader for {} ignored", msg.leader_port);
            return;
        }

        if let Some(current_port) = self.leader_port
            && current_port == msg.leader_port
        {
            election_debug!("Same leader {} being set again, ignoring", msg.leader_port);
            return;
        }

        leader_info!(
            "NEW LEADER ELECTED: {} (from pump {})",
            msg.leader_port,
            self.address
        );

        // Stop the election system FIRST to prevent more coordinator messages
        if let Some(election) = &self.election {
            election.receive_coordinator(msg.leader_port as usize);
            election.stop();
        }

        let old_leader = self.leader_port;

        // Update state - fix the leader address assignment
        self.leader_port = Some(msg.leader_port);
        self.is_leader = self.address.ends_with(&msg.leader_port.to_string());
        self.leader_assigned = true;

        // Always set leader_addr from addr_pumps, regardless of whether this pump is the leader
        self.leader_addr = self.addr_pumps.get(&msg.leader_port).cloned();

        if self.is_leader {
            leader_info!("I AM THE LEADER: {}", self.address);
            leader_info!("New leader connecting to Regional Admin");
            self.connect_to_regional_admin();
        } else {
            leader_info!(
                "Leader is: {} (address: {:?})",
                msg.leader_port,
                self.leader_addr
            );
            if self.regional_admin_connection.is_some() {
                leader_info!("No longer leader - disconnecting from Regional Admin");
                self.regional_admin_connection = None;
            }
        }

        self.election = None;

        if old_leader.is_some() && old_leader != Some(msg.leader_port) {
            leader_info!("Leader changed - recovering pending expenses");
            self.recover_pending_expenses();
        }
    }
}

impl Handler<Coordinator> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: Coordinator, _ctx: &mut Self::Context) -> Self::Result {
        leader_info!("Pump {} received Coordinator message", self.address);
        // Coordinator carries the port of the new leader; set local state accordingly
        let sender_port = _msg.sender_port;
        self.leader_port = Some(sender_port);
        self.is_leader = match self
            .address
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
        {
            Some(_) => {
                self.leader_assigned = true;
                let leader_addr_opt = self.addr_pumps.get(&sender_port).cloned();
                self.leader_addr = leader_addr_opt;
                true
            }
            None => false,
        };
    }
}

impl Handler<GetLeaderInfo> for Pump {
    type Result = Option<u32>;

    fn handle(&mut self, _msg: GetLeaderInfo, _ctx: &mut Self::Context) -> Self::Result {
        self.leader_port
    }
}

impl Handler<SetAddress> for Pump {
    type Result = ();

    fn handle(&mut self, msg: SetAddress, _ctx: &mut Self::Context) -> Self::Result {
        self.address = msg.address.clone();
        fuel_debug!("Pump saved its address: {}", self.address);

        if self.is_leader {
            leader_debug!("Leader pump is now listening on {}", self.address);
        }
    }
}

impl Handler<Start> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: Start, _ctx: &mut Self::Context) {
        if let Some(pump_addr) = self.self_addr.clone() {
            self.listen_for_card_connection(pump_addr);
        } else {
            error!(
                "Start received before self_addr was set for {}",
                self.address
            );
        }
    }
}

impl Handler<crate::message::SetSelfAddr> for Pump {
    type Result = ();

    fn handle(
        &mut self,
        msg: crate::message::SetSelfAddr,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        self.self_addr = Some(msg.addr);
    }
}

impl Handler<ExpenseConfirmed> for Pump {
    type Result = ();

    fn handle(&mut self, msg: ExpenseConfirmed, _ctx: &mut Self::Context) -> Self::Result {
        if msg.pump_id == self.get_my_id() {
            fuel_info!(" Expense confirmed by leader - clearing pending");
            self.buffered_expenses_pump.clear();
        }
    }
}

//Leader
impl Handler<Expense> for Pump {
    type Result = ResponseFuture<bool>;

    fn handle(&mut self, msg: Expense, _ctx: &mut Self::Context) -> Self::Result {
        if self.should_crash_for_operation(CrashableOperation::ExpenseForwarding) {
            crash_info!("CRASH while handling expense ${}", msg.total);
            self.simulate_crash();
            return Box::pin(async { false });
        }

        fuel_info!(
            "Station Leader with address {} received Expense: Company {}, Card {}, Total {}, Date {}",
            self.address,
            msg.company_id,
            msg.card_id,
            msg.total,
            msg.date
        );

        self.mailbox_expenses_lider.push_back(msg.clone());

        // Clone data for async block
        let regional_admin_connection = self.regional_admin_connection.clone();
        let pump_address = self.address.clone();
        let expense = msg.clone();
        let service_station_id = self.service_station_id;
        let addr_pumps = self.addr_pumps.clone();
        let my_id = self.get_my_id();
        let self_addr = self.self_addr.clone();
        let current_admin_addr = self
            .regional_admin_addrs
            .get(self.current_regional_admin_index)
            .cloned();
        let current_index = self.current_regional_admin_index;

        Box::pin(async move {
            if let Some(regional_conn) = regional_admin_connection {
                let regional_msg = RegionalAdminMessage::PumpExpense {
                    pump_address: pump_address.clone(),
                    expense: expense.clone(),
                    service_station_id: service_station_id.to_string(),
                };

                let send_msg = SendToRegionalAdmin {
                    msg_type: common::message::MsgType::PumpExpense,
                    content: regional_msg,
                };

                regional_admin_info!(
                    "Sending expense to Regional Admin: ${} for company {} (service_station: {})",
                    expense.total,
                    expense.company_id,
                    service_station_id
                );

                match regional_conn.send(send_msg).await {
                    Ok(_) => {
                        regional_admin_info!(
                            "Successfully sent expense to Regional Admin: ${} (service_station: {})",
                            expense.total,
                            service_station_id
                        );
                        // Response will come asynchronously via PaymentProcessedResponse
                        true
                    }
                    Err(e) => {
                        regional_admin_error!("Failed to send to Regional Admin: {}", e);
                        regional_admin_info!("Triggering failover to next Regional Admin");

                        // Trigger failover to next Regional Admin
                        if let (Some(self_addr), Some(failed_addr)) =
                            (self_addr, current_admin_addr)
                        {
                            self_addr.do_send(TryNextRegionalAdmin {
                                failed_address: failed_addr,
                                regional_admin_addrs: vec![], // Will be populated by the handler
                                current_index,
                            });

                            // Also disconnect the current connection
                            self_addr.do_send(DisconnectRegionalAdmin);
                        }

                        // Send failure to originating pump
                        if let Some(pump_addr) = addr_pumps.get(&expense.company_id) {
                            let confirmation = ExpenseConfirmed {
                                pump_id: my_id,
                                state: false,
                            };
                            pump_addr.do_send(confirmation);
                        }

                        false
                    }
                }
            } else {
                regional_admin_error!(
                    "No Regional Admin connection available for pump {}",
                    pump_address
                );

                if let Some(self_addr) = self_addr {
                    self_addr.do_send(crate::message::ConnectToRegionalAdmin);
                }

                // Send failure to originating pump when no connection
                if let Some(pump_addr) = addr_pumps.get(&expense.company_id) {
                    let confirmation = ExpenseConfirmed {
                        pump_id: my_id,
                        state: false,
                    };
                    pump_addr.do_send(confirmation);
                }

                false
            }
        })
    }
}

// Handler for ExpenseWithOrigin (from non-leader pumps to leader)
impl Handler<ExpenseWithOrigin> for Pump {
    type Result = ResponseFuture<bool>;

    fn handle(&mut self, msg: ExpenseWithOrigin, _ctx: &mut Self::Context) -> Self::Result {
        fuel_info!(
            "Leader pump received ExpenseWithOrigin from pump {}: card_id={}, total=${}",
            msg.originating_pump,
            msg.expense.card_id,
            msg.expense.total
        );

        if self.should_crash_for_operation(CrashableOperation::ExpenseForwarding) {
            crash_info!("CRASH while handling expense ${}", msg.expense.total);
            self.simulate_crash();
            return Box::pin(async { false });
        }

        fuel_info!(
            "Station Leader forwarding expense from pump {}: Company {}, Card {}, Total {}, Date {}",
            msg.originating_pump,
            msg.expense.company_id,
            msg.expense.card_id,
            msg.expense.total,
            msg.expense.date
        );

        self.mailbox_expenses_lider.push_back(msg.expense.clone());

        // Clone all needed values before moving into async block
        let regional_admin_connection = self.regional_admin_connection.clone();
        let pump_address = msg.originating_pump.clone();
        let expense = msg.expense.clone();
        let service_station_id = self.service_station_id;
        let addr_pumps = self.addr_pumps.clone();
        let my_id = self.get_my_id();
        let self_addr = self.self_addr.clone(); // Clone instead of moving
        let current_admin_addr = self
            .regional_admin_addrs
            .get(self.current_regional_admin_index)
            .cloned();
        let current_index = self.current_regional_admin_index;

        Box::pin(async move {
            if let Some(regional_conn) = regional_admin_connection {
                let regional_msg = RegionalAdminMessage::PumpExpense {
                    pump_address: pump_address.clone(),
                    expense: expense.clone(),
                    service_station_id: service_station_id.to_string(),
                };

                let send_msg = SendToRegionalAdmin {
                    msg_type: common::message::MsgType::PumpExpense,
                    content: regional_msg,
                };

                regional_admin_info!(
                    "Sending expense to Regional Admin: ${} for company {} (service_station: {}, originating_pump: {})",
                    expense.total,
                    expense.company_id,
                    service_station_id,
                    pump_address
                );

                match regional_conn.send(send_msg).await {
                    Ok(_) => {
                        regional_admin_info!(
                            "Successfully sent expense to Regional Admin: ${} (service_station: {}, originating_pump: {})",
                            expense.total,
                            service_station_id,
                            pump_address
                        );
                        true
                    }
                    Err(e) => {
                        regional_admin_error!("Failed to send to Regional Admin: {}", e);
                        regional_admin_info!("Triggering failover to next Regional Admin");

                        if let (Some(self_addr), Some(failed_addr)) =
                            (self_addr.as_ref(), current_admin_addr)
                        {
                            self_addr.do_send(TryNextRegionalAdmin {
                                failed_address: failed_addr,
                                regional_admin_addrs: vec![],
                                current_index,
                            });
                            self_addr.do_send(DisconnectRegionalAdmin);
                        }

                        if let Some(pump_addr) = addr_pumps.get(&expense.company_id) {
                            let confirmation = ExpenseConfirmed {
                                pump_id: my_id,
                                state: false,
                            };
                            pump_addr.do_send(confirmation);
                        }

                        false
                    }
                }
            } else {
                regional_admin_error!("No Regional Admin connection available for leader pump");

                if let Some(self_addr) = self_addr.as_ref() {
                    self_addr.do_send(crate::message::ConnectToRegionalAdmin);
                }

                if let Some(pump_addr) = addr_pumps.get(&expense.company_id) {
                    let confirmation = ExpenseConfirmed {
                        pump_id: my_id,
                        state: false,
                    };
                    pump_addr.do_send(confirmation);
                }

                false
            }
        })
    }
}

/// Types of operations that can crash
#[derive(Debug, Clone, Copy)]
enum CrashableOperation {
    ElectionProcessing,
    ExpenseForwarding,
}

/// Recovery message after crash
#[derive(Debug, Clone)]
pub struct CrashRecovery;

impl actix::Message for CrashRecovery {
    type Result = ();
}

impl Handler<CrashRecovery> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: CrashRecovery, _ctx: &mut Self::Context) -> Self::Result {
        crash_info!("PUMP {} RECOVERING FROM CRASH", self.address);

        // Reinitialize basic state (but don't assume leadership)
        self.leader_assigned = false;
        self.is_leader = false;
        self.leader_port = None;
        self.leader_addr = None;

        // Reinitialize election system
        if !self.addr_pumps.is_empty() {
            self.initialize_election_channel();

            // Wait a bit, then trigger election to rejoin network
            if let Some(election) = &self.election {
                let election_clone = election.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                    election_clone.trigger_election();
                });
            }
        }

        crash_info!(" PUMP {} RECOVERY COMPLETE", self.address);
    }
}

/// Random crash trigger message
#[derive(Debug, Clone)]
pub struct RandomCrashTrigger;

impl actix::Message for RandomCrashTrigger {
    type Result = ();
}

impl Handler<RandomCrashTrigger> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: RandomCrashTrigger, _ctx: &mut Self::Context) -> Self::Result {
        if self.should_crash() {
            crash_info!("RANDOM CRASH TRIGGERED for pump {}", self.address);
            self.simulate_crash();
        }

        for pump_addr in self.addr_pumps.values() {
            let confirmation = ExpenseConfirmed {
                pump_id: self.get_my_id(),
                state: true,
            };
            pump_addr.do_send(confirmation);
        }
    }
}

impl Handler<StoreRegionalAdminConnection> for Pump {
    type Result = ();

    fn handle(
        &mut self,
        msg: StoreRegionalAdminConnection,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        regional_admin_info!(
            "Regional Admin connection established for pump {}",
            self.address
        );
        self.regional_admin_connection = Some(msg.connection);
    }
}

impl Handler<ConnectToRegionalAdmin> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: ConnectToRegionalAdmin, _ctx: &mut Self::Context) -> Self::Result {
        regional_admin_info!(
            "🔄 Pump {} received ConnectToRegionalAdmin message (current_index: {})",
            self.address,
            self.current_regional_admin_index
        );

        // Only connect if we're a leader
        if !self.is_leader {
            regional_admin_info!(
                "Pump {} is not leader, ignoring ConnectToRegionalAdmin",
                self.address
            );
            return;
        }

        // If we already have a connection, log but continue (might be a retry)
        if self.regional_admin_connection.is_some() {
            regional_admin_info!(
                "Pump {} already has Regional Admin connection, will attempt new one",
                self.address
            );
            self.regional_admin_connection = None; // Clear existing connection
        }

        self.connect_to_regional_admin();
    }
}

//-------------------------------Admin rgional coneccion --------

pub struct PumpRegionalAdminConnection {
    writer: Option<OwnedWriteHalf>,
    pump_addr: Addr<Pump>,
    reader_task_cancel_token: Option<CancellationToken>,
}

impl PumpRegionalAdminConnection {
    pub fn new(writer: OwnedWriteHalf, pump_addr: Addr<Pump>) -> Self {
        Self {
            writer: Some(writer),
            pump_addr,
            reader_task_cancel_token: None,
        }
    }

    fn spawn_reader_task(&mut self, mut reader: OwnedReadHalf, cancel_token: CancellationToken) {
        let pump_addr_clone = self.pump_addr.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        regional_admin_info!("🔌 Regional Admin reader task cancelled gracefully");
                        break;
                    }

                    msg_type_result = TCPProtocol::receive_msg_type_from_reader(&mut reader) => {
                        match msg_type_result {
                            Ok(MsgType::PaymentProcessed) => {
                                match TCPProtocol::receive_msg_from_reader::<common::message::CompanyMessage>(&mut reader).await {
                                    Ok(common::message::CompanyMessage::PaymentProcessed { accepted, expense, pump_address, .. }) => {
                                        regional_admin_info!(
                                            "✅ Pump received PaymentProcessed from Regional Admin: accepted={}, total=${}, pump_address={}",
                                            accepted,
                                            expense.total,
                                            pump_address
                                        );
                                        pump_addr_clone.do_send(PaymentProcessedResponse { accepted, expense, pump_address });
                                    }
                                    Err(e) => {
                                        regional_admin_error!(
                                            "❌ Failed to parse PaymentProcessed message: {}",
                                            e
                                        );
                                        pump_addr_clone.do_send(TriggerRegionalAdminFailover);
                                        break;
                                    }
                                }
                            }
                            Ok(other_type) => {
                                regional_admin_error!(
                                    "❌ Unexpected message type from Regional Admin: {:?}",
                                    other_type
                                );
                            }
                            Err(e) => {
                                regional_admin_error!("❌ Connection to Regional Admin closed: {}", e);
                                pump_addr_clone.do_send(TriggerRegionalAdminFailover);
                                break;
                            }
                        }
                    }
                }
            }
            regional_admin_info!("🔌 Regional Admin reader task terminated");
        });
    }
}

impl Actor for PumpRegionalAdminConnection {
    type Context = Context<Self>;

    fn stopping(&mut self, _ctx: &mut Self::Context) -> actix::Running {
        if let Some(token) = &self.reader_task_cancel_token {
            token.cancel();
            regional_admin_info!("Cancelling PumpRegionalAdminConnection reader task...");
        }
        actix::Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        regional_admin_info!("PumpRegionalAdminConnection actor stopped");
    }
}

impl Handler<crate::message::SendToRegionalAdmin> for PumpRegionalAdminConnection {
    type Result = ResponseFuture<()>;

    fn handle(
        &mut self,
        msg: crate::message::SendToRegionalAdmin,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        if let Some(mut writer) = self.writer.take() {
            let msg_type = msg.msg_type;
            let pump_addr = self.pump_addr.clone();
            let self_addr = _ctx.address();

            Box::pin(async move {
                regional_admin_info!(
                    "📤 Sending expense message to Regional Admin: {:?}",
                    msg_type
                );
                let send_result =
                    TCPProtocol::send_with_writer(&mut writer, msg_type.clone(), &msg.content)
                        .await;

                if let Err(e) = send_result {
                    regional_admin_error!("❌ Failed to send expense to Regional Admin: {}", e);
                    regional_admin_info!(
                        "🔄 Regional Admin communication failure detected - triggering failover"
                    );
                    pump_addr.do_send(TriggerRegionalAdminFailover);
                } else {
                    // Restore writer via a message to self
                    self_addr.do_send(RestoreWriter { writer });
                }
            })
        } else {
            regional_admin_error!("❌ No TCP writer available for Regional Admin");
            self.pump_addr.do_send(TriggerRegionalAdminFailover);
            Box::pin(async {})
        }
    }
}

// Helper message to restore writer
struct RestoreWriter {
    writer: OwnedWriteHalf,
}

impl actix::Message for RestoreWriter {
    type Result = ();
}

impl Handler<RestoreWriter> for PumpRegionalAdminConnection {
    type Result = ();

    fn handle(&mut self, msg: RestoreWriter, _ctx: &mut Self::Context) -> Self::Result {
        self.writer = Some(msg.writer);
    }
}

// Add new message type for triggering failover
#[derive(Debug, Clone)]
pub struct TriggerRegionalAdminFailover;

impl actix::Message for TriggerRegionalAdminFailover {
    type Result = ();
}

impl Handler<TriggerRegionalAdminFailover> for Pump {
    type Result = ();

    fn handle(
        &mut self,
        _msg: TriggerRegionalAdminFailover,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        regional_admin_info!(
            "Pump {} received Regional Admin failover trigger",
            self.address
        );

        // Get current failed address
        let failed_address = self
            .regional_admin_addrs
            .get(self.current_regional_admin_index)
            .cloned();
        let current_index = self.current_regional_admin_index;

        // Disconnect current connection
        self.regional_admin_connection = None;

        // Try next regional admin
        if let Some(failed_addr) = failed_address {
            self.try_next_regional_admin(failed_addr, current_index);
        } else {
            regional_admin_error!("No failed address available for failover");
        }
    }
}

impl Handler<crate::message::PaymentProcessedResponse> for Pump {
    type Result = ();

    fn handle(
        &mut self,
        msg: crate::message::PaymentProcessedResponse,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        regional_admin_info!(
            "✅ Pump {} received PaymentProcessed response: accepted={}, total=${}, card_id={}, target_pump={}",
            self.address,
            msg.accepted,
            msg.expense.total,
            msg.expense.card_id,
            msg.pump_address
        );

        // Check if this pump has the card connection (direct channel)
        if let Some(sender) = self.card_response_channels.remove(&msg.expense.card_id) {
            fuel_info!(
                "Sending payment result to waiting card {}: success={}",
                msg.expense.card_id,
                msg.accepted
            );

            if sender.send(msg.accepted).is_err() {
                error!(
                    "Failed to send payment result to card {} - channel closed, storing as pending",
                    msg.expense.card_id
                );

                let pending_response = GasPumpYPFMessage::FuelComplete {
                    success: msg.accepted,
                };
                self.pending_card_results
                    .insert(msg.expense.card_id, (pending_response, String::new()));
                fuel_info!(
                    "Stored pending result for card {} after channel closure",
                    msg.expense.card_id
                );
            }
        } else if msg.pump_address == self.address {
            regional_admin_error!(
                "Received PaymentProcessed for card {} but no response channel found (card may have disconnected)",
                msg.expense.card_id
            );
            let pending_response = GasPumpYPFMessage::FuelComplete {
                success: msg.accepted,
            };
            self.pending_card_results
                .insert(msg.expense.card_id, (pending_response, String::new()));
            fuel_info!(
                "Stored pending result for card {} - no active channel",
                msg.expense.card_id
            );
        } else {
            regional_admin_info!(
                "Leader forwarding PaymentProcessed to pump {} for card {}",
                msg.pump_address,
                msg.expense.card_id
            );

            #[allow(clippy::collapsible_if)]
            if let Some(port_str) = msg.pump_address.rsplit(':').next() {
                if let Ok(port) = port_str.parse::<u32>() {
                    if let Some(pump_addr) = self.addr_pumps.get(&port) {
                        pump_addr.do_send(msg.clone());
                        fuel_info!(
                            "✅ Successfully forwarded PaymentProcessed to pump {}",
                            msg.pump_address
                        );
                    } else {
                        regional_admin_error!(
                            "Failed to find pump with port {} to forward PaymentProcessed",
                            port
                        );
                        let pending_response = GasPumpYPFMessage::FuelComplete {
                            success: msg.accepted,
                        };
                        self.pending_card_results
                            .insert(msg.expense.card_id, (pending_response, String::new()));
                        fuel_info!(
                            "Stored pending result for card {} - pump not found",
                            msg.expense.card_id
                        );
                    }
                } else {
                    regional_admin_error!(
                        "Failed to parse port from pump_address: {}",
                        msg.pump_address
                    );
                    let pending_response = GasPumpYPFMessage::FuelComplete {
                        success: msg.accepted,
                    };
                    self.pending_card_results
                        .insert(msg.expense.card_id, (pending_response, String::new()));
                }
            } else {
                regional_admin_error!("Invalid pump_address format: {}", msg.pump_address);
                let pending_response = GasPumpYPFMessage::FuelComplete {
                    success: msg.accepted,
                };
                self.pending_card_results
                    .insert(msg.expense.card_id, (pending_response, String::new()));
            }
        }
    }
}

impl Handler<GetPendingCardResult> for Pump {
    type Result = Option<GasPumpYPFMessage>;

    fn handle(&mut self, msg: GetPendingCardResult, _ctx: &mut Self::Context) -> Self::Result {
        self.pending_card_results
            .get(&msg.card_id)
            .map(|(msg, _addr)| msg.clone())
    }
}

impl Handler<RemovePendingCardResult> for Pump {
    type Result = ();

    fn handle(&mut self, msg: RemovePendingCardResult, _ctx: &mut Self::Context) -> Self::Result {
        self.pending_card_results.remove(&msg.card_id);
    }
}

impl Handler<StorePendingCardResult> for Pump {
    type Result = ();

    fn handle(&mut self, msg: StorePendingCardResult, _ctx: &mut Self::Context) -> Self::Result {
        self.pending_card_results
            .insert(msg.card_id, (msg.result, msg.card_address));
    }
}

// Message for storing card response channel
pub struct StoreCardResponseChannel {
    pub card_id: u32,
    pub sender: tokio::sync::oneshot::Sender<bool>,
}

impl actix::Message for StoreCardResponseChannel {
    type Result = ();
}

impl Handler<StoreCardResponseChannel> for Pump {
    type Result = ();

    fn handle(&mut self, msg: StoreCardResponseChannel, _ctx: &mut Self::Context) -> Self::Result {
        fuel_debug!("Storing response channel for card {}", msg.card_id);
        self.card_response_channels.insert(msg.card_id, msg.sender);
    }
}

impl Handler<TryNextRegionalAdmin> for Pump {
    type Result = ();

    fn handle(&mut self, msg: TryNextRegionalAdmin, _ctx: &mut Self::Context) -> Self::Result {
        // Use the pump's regional_admin_addrs instead of the message's
        self.try_next_regional_admin(msg.failed_address, msg.current_index);
    }
}

// Add new message for disconnecting Regional Admin
#[derive(Debug, Clone)]
pub struct DisconnectRegionalAdmin;

impl actix::Message for DisconnectRegionalAdmin {
    type Result = ();
}

impl Handler<DisconnectRegionalAdmin> for Pump {
    type Result = ();

    fn handle(&mut self, _msg: DisconnectRegionalAdmin, _ctx: &mut Self::Context) -> Self::Result {
        regional_admin_info!("Disconnecting from current Regional Admin");
        self.regional_admin_connection = None;
    }
}

impl Handler<Election> for Pump {
    type Result = ();

    fn handle(&mut self, msg: Election, ctx: &mut Self::Context) -> Self::Result {
        // Check if addr_pumps is empty and defer if needed
        if self.addr_pumps.is_empty() {
            election_info!(
                "Pump {} not ready for election (no addr_pumps), deferring",
                self.address
            );
            let msg_copy = msg.clone();
            ctx.run_later(Duration::from_millis(500), move |act, ctx| {
                if !act.addr_pumps.is_empty() {
                    election_info!("Retrying deferred Election for pump {}", act.address);
                    let _ = ctx.address().try_send(msg_copy);
                } else {
                    error!("Pump {} still not ready after defer", act.address);
                }
            });
            return;
        }

        // Handle crash simulation safely
        if !self.crash_safe_election_processing(&msg) {
            return; // Crashed during processing
        }

        let should_ignore = self.should_ignore_election(&msg);
        election_info!(
            "Pump {} received Election from {} (should_ignore={})",
            self.address,
            msg.sender_port,
            should_ignore
        );

        if should_ignore {
            election_info!("Election ignored by pump {}", self.address);
            return;
        }

        // Special handling for leader failure notification (sender_port = 0)
        if msg.sender_port == 0 {
            election_info!("Leader failure detected - triggering emergency election");
            self.trigger_emergency_election();
            return;
        }

        if self.leader_assigned && self.leader_addr.is_some() {
            election_info!("Ignoring election - leader already stable");
            return;
        }

        // Ensure election system is initialized
        if !self.ensure_election_initialized() {
            election_info!("Failed to initialize election for pump {}", self.address);
            return;
        }

        // Apply debouncing to prevent spam
        if self.is_election_too_recent() {
            election_info!("Election too recent for pump {}", self.address);
            return;
        }

        election_info!("Processing election message for pump {}", self.address);
        self.process_bully_election(msg.sender_port);
        self.last_election = Some(Instant::now());
    }
}

impl Handler<TcpFuelRequest> for Pump {
    type Result = ResponseFuture<bool>;

    fn handle(&mut self, msg: TcpFuelRequest, _ctx: &mut Self::Context) -> Self::Result {
        fuel_info!(
            "Pump {} processing TCP fuel request for card {} ({} liters)",
            self.address,
            msg.request.card_id,
            msg.request.liters
        );

        // Validate request
        if msg.request.liters == 0 {
            fuel_info!("Invalid request: liters must be > 0");
            return Box::pin(async { false });
        }

        // Check if we have a leader - with retry mechanism for initialization
        if !self.leader_assigned || self.leader_addr.is_none() {
            fuel_info!(
                "No leader available for pump {} - triggering election and waiting",
                self.address
            );

            // Trigger emergency election
            self.trigger_emergency_election();

            // Return false for now - the card will need to retry
            fuel_info!("First request denied - election in progress, card should retry");
            return Box::pin(async { false });
        }

        // Store the request in buffer
        let expense = Expense {
            company_id: msg.request.company_id,
            card_id: msg.request.card_id,
            total: msg.request.liters * PRICEPERLITER,
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        };

        self.buffered_expenses_pump.push_back(expense.clone());

        let leader_addr = self.leader_addr.clone();
        let originating_pump = self.address.clone();
        let self_addr = self.self_addr.clone();

        Box::pin(async move {
            if let Some(leader) = leader_addr {
                match leader
                    .send(ExpenseWithOrigin {
                        expense,
                        originating_pump,
                    })
                    .await
                {
                    Ok(ack) => {
                        fuel_info!("Expense forwarded to leader successfully");
                        ack
                    }
                    Err(e) => {
                        error!("Failed to send expense to leader: {}", e);

                        // Trigger election if leader communication fails
                        if let Some(self_addr) = self_addr {
                            self_addr.do_send(Election { sender_port: 0 });
                        }
                        false
                    }
                }
            } else {
                error!("No leader address available");
                false
            }
        })
    }
}
