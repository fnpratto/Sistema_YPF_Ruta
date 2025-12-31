use actix::{Addr, Message};
use common::message::{Expense, MsgType, RegionalAdminMessage};
use serde::{Deserialize, Serialize};

use crate::connection_actor::RegionalAdminConnectionActor;

/*
----------------------------------------------
                ACTOR MESSAGES
----------------------------------------------
*/

#[allow(dead_code)]
#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct PumpExpense {
    pub pump_address: String,
    pub expense: Expense,
    pub service_station_id: String,
    pub connection_actor: Option<Addr<RegionalAdminConnectionActor>>,
}

impl PumpExpense {
    pub fn new(pump_address: String, expense: Expense, service_station_id: String) -> Self {
        Self {
            pump_address,
            expense,
            service_station_id,
            connection_actor: None,
        }
    }

    pub fn with_connection(
        pump_address: String,
        expense: Expense,
        service_station_id: String,
        connection_actor: Addr<RegionalAdminConnectionActor>,
    ) -> Self {
        Self {
            pump_address,
            expense,
            service_station_id,
            connection_actor: Some(connection_actor),
        }
    }
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize, Debug)]
#[rtype(result = "()")]
pub struct CompanyPaymentProcessed {
    pub pump_address: String,
    pub expense: Expense,
    pub accepted: bool,
    pub service_station_id: String,
}

impl CompanyPaymentProcessed {
    pub fn new(
        pump_address: String,
        expense: Expense,
        accepted: bool,
        service_station_id: String,
    ) -> Self {
        Self {
            pump_address,
            expense,
            accepted,
            service_station_id,
        }
    }
}

#[allow(dead_code)]
#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct StoreCompanyConnection {
    pub connection: Addr<RegionalAdminConnectionActor>,
}

#[allow(dead_code)]
#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct PumpConnection {
    pub pump_address: String,
    pub service_station_id: String,
    pub connection: Addr<RegionalAdminConnectionActor>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SendToClient<T: Serialize + Send + 'static> {
    pub msg_type: MsgType,
    pub msg: T,
}

impl SendToClient<RegionalAdminMessage> {
    pub fn new(msg_type: MsgType, msg: RegionalAdminMessage) -> Self {
        Self { msg_type, msg }
    }
}

#[allow(dead_code)]
#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct SendToPumpFailed {
    pub pump_address: String,
    pub service_station_id: String,
    pub msg_type: MsgType,
    pub expense: Expense,
    pub accepted: bool,
}

#[allow(dead_code)]
#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct SendToPumpSuccess {
    pub pump_address: String,
    pub expense: Expense,
}

#[allow(dead_code)]
#[derive(Message)]
#[rtype(result = "()")]
pub struct SendPaymentProcessedToPump {
    pub pump_address: String,
    pub expense: Expense,
    pub accepted: bool,
    pub service_station_id: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SendToCompanyFailed {
    pub message: RegionalAdminMessage,
}
