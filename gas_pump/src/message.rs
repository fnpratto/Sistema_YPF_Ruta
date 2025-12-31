use std::collections::HashMap;

use actix::Addr;
use actix::prelude::*;
use common::message::RegionalAdminMessage;
// For Message, Handler traits and rtype attribute
use crate::pump::{Pump, PumpRegionalAdminConnection};
use common::message::Expense;
use common::message::MsgType;
use serde::{Deserialize, Serialize}; // If you need serialization

// Message used to update a pump with the current map of pump addresses
pub struct UpdatePumps {
    pub addr_pumps: HashMap<u32, Addr<Pump>>,
}

impl actix::Message for UpdatePumps {
    type Result = ();
}

#[derive(Clone)]
pub struct Election {
    pub sender_port: u32,
}

#[derive(Debug, Clone)]
pub struct ExpenseConfirmed {
    pub pump_id: usize,
    pub state: bool,
}

impl actix::Message for ExpenseConfirmed {
    type Result = ();
}

pub struct Coordinator {
    pub sender_port: u32,
}

pub struct SetLeader {
    pub leader_port: u32,
}

pub struct GetLeaderInfo {
    pub requester_port: u32,
}

// Message sent from the TCP listener task to the pump actor carrying a
// parsed fuel request received over TCP (from a card). The pump actor
// will process it (forward to leader) and return an ack boolean.
pub struct TcpFuelRequest {
    pub request: common::message::FuelRequestData,
    pub card_address: String,
}

impl actix::Message for SetLeader {
    type Result = ();
}
impl actix::Message for GetLeaderInfo {
    type Result = Option<u32>;
}

impl actix::Message for TcpFuelRequest {
    type Result = bool;
}

// Internal message used to inform the actor about its own Addr
pub struct SetSelfAddr {
    pub addr: Addr<crate::pump::Pump>,
}

impl actix::Message for SetSelfAddr {
    type Result = ();
}

impl actix::Message for Election {
    type Result = ();
}
impl actix::Message for Coordinator {
    type Result = ();
}
//------------------

#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct StoreRegionalAdminConnection {
    pub connection: Addr<PumpRegionalAdminConnection>,
}

#[derive(Message, Debug, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct ConnectToRegionalAdmin;

#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct SendToRegionalAdmin {
    pub msg_type: MsgType,
    pub content: RegionalAdminMessage,
}

impl SendToRegionalAdmin {
    pub fn new(msg_type: MsgType, content: RegionalAdminMessage) -> Self {
        Self { msg_type, content }
    }
}

// In your message.rs file
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
#[allow(dead_code)]
pub struct SendToClient<T> {
    pub content: T,
}
/// QUIC messages for pump-to-pump communication
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum PumpMessage {
    /// Election message sent via QUIC
    Election { sender_port: u32 },

    /// Coordinator message sent via QUIC
    Coordinator { sender_port: u32 },

    /// Expense forwarding via QUIC
    Expense(Expense),

    /// Leader failure notification
    LeaderFailure {
        failed_leader_port: u32,
        sender_port: u32,
    },
}

/// QUIC connection request message
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct QuicConnectionRequest {
    pub sender_port: u32,
    pub connection_type: QuicConnectionType,
    pub timestamp: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum QuicConnectionType {
    Election,
    ExpenseForwarding,
    Heartbeat,
    StatusCheck,
    Emergency,
}

/// QUIC response wrapper
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct QuicResponse<T> {
    pub sender_port: u32,
    pub success: bool,
    pub data: Option<T>,
    pub error_message: Option<String>,
    pub timestamp: u64,
}

#[derive(Message, Debug, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct TryNextRegionalAdmin {
    pub failed_address: String,
    pub regional_admin_addrs: Vec<String>,
    pub current_index: usize,
}

#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct PaymentProcessedResponse {
    pub accepted: bool,
    pub expense: Expense,
    pub pump_address: String,
}

#[derive(Message, Debug, Clone)]
#[rtype(result = "bool")]
pub struct ExpenseWithOrigin {
    pub expense: Expense,
    pub originating_pump: String,
}
