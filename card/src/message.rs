use actix::Message;
use common::message::FuelRequestData;
use serde::{Deserialize, Serialize};

/*
----------------------------------------------
                ACTOR MESSAGES
----------------------------------------------
*/

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct FuelRequest {
    pub data: FuelRequestData,
}

#[allow(dead_code)]
impl FuelRequest {
    pub fn new(company_id: u32, card_id: u32, liters: u32) -> Self {
        Self {
            data: common::message::FuelRequestData {
                company_id,
                card_id,
                liters,
            },
        }
    }
}

#[allow(dead_code)]
#[derive(Message, Clone, Serialize, Deserialize)]
#[rtype(result = "()")]
pub struct FuelComplete {
    pub success: bool,
}

#[allow(dead_code)]
impl FuelComplete {
    pub fn new(success: bool) -> Self {
        Self { success }
    }
}
