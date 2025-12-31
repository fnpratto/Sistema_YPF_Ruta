use actix::Message;
use serde::{Deserialize, Serialize};

/*
 * Msg types used in the TCP Protocol as operation codes.
 * We could put #[repr(u8)] here for better serialization operation codes
 */
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum MsgType {
    FuelRequest = 0,
    FuelComplete = 1,
    GetPendingResult = 2,
    NoPendingResult = 3,
    PumpExpense = 4,
    PaymentProcessed = 5,
    CardReconnect = 6,
    PaymentProcessedAck = 7,
    AskTotalBalance = 10,
    ChargeBalance = 11,
    RegisterNewCard = 12,
    CheckCardLimit = 13,
    EstablishCardBalance = 14,
    CheckAllCards = 15,
    CheckExpenses = 16,
    GetPendingCardResult = 17,
    ServiceStationReconnected = 18,
}

impl MsgType {
    #[allow(dead_code)]
    pub fn from_u8(value: u8) -> std::io::Result<Self> {
        match value {
            0 => Ok(MsgType::FuelRequest),
            1 => Ok(MsgType::FuelComplete),
            2 => Ok(MsgType::GetPendingResult),
            3 => Ok(MsgType::NoPendingResult),
            4 => Ok(MsgType::PumpExpense),
            5 => Ok(MsgType::PaymentProcessed),
            6 => Ok(MsgType::CardReconnect),
            7 => Ok(MsgType::PaymentProcessedAck),
            10 => Ok(MsgType::AskTotalBalance),
            11 => Ok(MsgType::ChargeBalance),
            12 => Ok(MsgType::RegisterNewCard),
            13 => Ok(MsgType::CheckCardLimit),
            14 => Ok(MsgType::EstablishCardBalance),
            15 => Ok(MsgType::CheckAllCards),
            16 => Ok(MsgType::CheckExpenses),
            17 => Ok(MsgType::GetPendingCardResult),
            18 => Ok(MsgType::ServiceStationReconnected),
            _ => Err(std::io::Error::other(format!("Invalid MsgType: {}", value))),
        }
    }
}

/*
----------------------------------------------
              PROTOCOL MESSAGES
----------------------------------------------
*/

/*-------------------CARD MESSAGES------------------------------- */
/*
 * Common request data structure for protocol messages.
 */
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FuelRequestData {
    pub company_id: u32,
    pub card_id: u32,
    pub liters: u32,
}

/*
 * Messages used in the TCP protocol between GasPump and Card.
 */
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GasPumpYPFMessage {
    FuelRequest {
        request: FuelRequestData,
        card_address: String,
    },
    FuelComplete {
        success: bool,
    },
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StorePendingCardResult {
    pub card_id: u32,
    pub card_address: String,
    pub result: GasPumpYPFMessage,
}

impl actix::Message for StorePendingCardResult {
    type Result = ();
}

/*
 * Messages used in the TCP protocol between GasPump and RegionalAdmin.
 */
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RegionalAdminMessage {
    PumpExpense {
        pump_address: String,
        expense: Expense,
        service_station_id: String,
    },
    PaymentProcessedAck {
        pump_address: String,
        expense: Expense,
    },
    ServiceStationReconnected {
        service_station_id: String,
        regional_admin_id: String,
    },
}

/*
 * Messages used in the TCP protocol between Company and RegionalAdmin.
 */
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum CompanyMessage {
    PaymentProcessed {
        pump_address: String,
        expense: Expense,
        accepted: bool,
        service_station_id: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct CardReconnect {
    pub card_id: u32,
}

/*-------------------PUMP MESSAGES------------------------------- */

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub enum MessagesPump {
    Start,
    RefuelRequest {
        op_code: u32,
        request: FuelRequest,
        card_address: String,
    },
    Expense(Expense),
    SetAddress(SetAddress),
}

// Implement the Message trait for ServiceStationYPFMessage
impl Message for MessagesPump {
    type Result = ();
}
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct FuelRequest {
    pub company_id: u32,
    pub card_id: u32,
    pub liters: u32,
}

#[allow(dead_code)]
impl FuelRequest {
    pub fn new(company_id: u32, card_id: u32, liters: u32) -> Self {
        Self {
            company_id,
            card_id,
            liters,
        }
    }
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct NotifyAvailability;

impl Message for NotifyAvailability {
    type Result = bool;
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct Start;

impl Message for Start {
    type Result = ();
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct RefuelRequest {
    pub op_code: u32,
    pub request: FuelRequest,
    pub card_address: String,
}

impl Message for RefuelRequest {
    type Result = ();
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Expense {
    pub company_id: u32,
    pub card_id: u32,
    pub total: u32,
    pub date: String,
}

#[allow(dead_code)]
impl Expense {
    pub fn new(company_id: u32, card_id: u32, total: u32, date: String) -> Self {
        Self {
            company_id,
            card_id,
            total,
            date,
        }
    }
}
impl Message for Expense {
    type Result = bool;
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
pub struct SetAddress {
    pub address: String,
}

impl Message for SetAddress {
    type Result = ();
}

/*--------COMPANY ADMIN CLIENT / COMPANY HANDLER MESSAGES-------- */
#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct AskTotalBalance;

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct ChargeBalance {
    pub amount: u32,
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct RegisterNewCard {
    pub card_id: String,
    pub amount: u32,
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct CheckCardLimit {
    pub card_id: String,
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct EstablishCardBalance {
    pub card_id: String,
    pub amount: u32,
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct CheckAllCards;

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct CheckExpenses;

#[allow(dead_code)]
#[derive(Message, Debug, Clone, Serialize, Deserialize)]
#[rtype(result = "Option<GasPumpYPFMessage>")]
pub struct GetPendingCardResult {
    pub card_id: u32,
}

#[allow(dead_code)]
#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub struct RemovePendingCardResult {
    pub card_id: u32,
}
