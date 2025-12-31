use actix::{Actor, ActorFutureExt, AsyncContext, Context, Handler, System, WrapFuture};
use common::message::{GasPumpYPFMessage, MsgType};
use common::tcp_protocol::TCPProtocol;
use log::debug;
use std::io::Result;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::message::{FuelComplete, FuelRequest};

/*
 * Card Actor:
 *     - Connects to a gas pump via TCP.
 *     - Sends fuel request messages to the gas pump.
 *     - Receives fuel completion notifications from the gas pump.
 */
#[allow(dead_code)]
pub struct Card {
    company_id: u32,
    card_id: u32,
    gas_pump_address: String,
    card_address: String,
    fuel_complete_sender: mpsc::UnboundedSender<FuelComplete>,
    tcp_writer: Option<OwnedWriteHalf>,
    reader_task_cancel_token: Option<CancellationToken>,
}

impl Card {
    pub fn new(
        company_id: u32,
        card_id: u32,
        gas_pump_address: &str,
        fuel_complete_sender: mpsc::UnboundedSender<FuelComplete>,
    ) -> Result<Self> {
        Ok(Self {
            company_id,
            card_id,
            gas_pump_address: gas_pump_address.to_string(),
            card_address: format!("{}:{}", "127.0.0.1", card_id + 40000),
            fuel_complete_sender,
            tcp_writer: None,
            reader_task_cancel_token: None,
        })
    }

    fn spawn_reader_task(
        &self,
        mut reader: tokio::net::tcp::OwnedReadHalf,
        cancel_token: CancellationToken,
        card_addr: actix::Addr<Card>,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("Card reader task cancelled gracefully");
                        break;
                    }

                    msg_type_result = TCPProtocol::receive_msg_type_from_reader(&mut reader) => {
                        match msg_type_result {
                            Ok(msg_type) => match msg_type {
                                MsgType::FuelComplete => {
                                    match TCPProtocol::receive_msg_from_reader::<GasPumpYPFMessage>(&mut reader).await {
                                        Ok(GasPumpYPFMessage::FuelComplete { success }) => {
                                            let fuel_complete = FuelComplete {success };
                                            card_addr.do_send(fuel_complete);
                                            break;
                                        }
                                        Ok(_) => {
                                            debug!("Unexpected message variant for FuelComplete");
                                        }
                                        Err(e) => {
                                            debug!("Error parsing FuelComplete: {}", e);
                                            break;
                                        }
                                    }
                                }
                                _ => {
                                    debug!("Unexpected MsgType received: {:?}", msg_type);
                                }
                            },
                            Err(_) => {
                                debug!("Connection to gas pump closed");
                                break;
                            }
                        }
                    }
                }
            }
            debug!("Card TCP reader task terminated");
        });
    }
}

impl Actor for Card {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let gas_pump_address = self.gas_pump_address.clone();
        let card_id = self.card_id;

        let fut = async move {
            // Small delay to allow any existing connections to close
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            debug!("Card {} connecting to pump {}", card_id, gas_pump_address);
            TCPProtocol::new(&gas_pump_address).await
        }
        .into_actor(self)
        .map(|result, actor, ctx| match result {
            Ok(tcp_protocol) => {
                let (reader, writer) = tcp_protocol.into_split();
                actor.tcp_writer = Some(writer);

                let cancel_token = CancellationToken::new();
                actor.reader_task_cancel_token = Some(cancel_token.clone());

                let card_addr = ctx.address();
                actor.spawn_reader_task(reader, cancel_token, card_addr);
            }
            Err(e) => {
                debug!(
                    "Failed to connect to gas pump at {}: {}",
                    actor.gas_pump_address, e
                );
                debug!("Terminating program - cannot proceed without gas pump connection");
                System::current().stop();
                std::process::exit(1);
            }
        });

        ctx.wait(fut);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> actix::Running {
        if let Some(token) = &self.reader_task_cancel_token {
            token.cancel();
            debug!("Cancelling Card reader task...");
        }
        actix::Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        debug!("Card actor stopped");
    }
}

impl Handler<FuelRequest> for Card {
    type Result = ();

    fn handle(&mut self, msg: FuelRequest, ctx: &mut Self::Context) -> Self::Result {
        let fuel_request: GasPumpYPFMessage = GasPumpYPFMessage::FuelRequest {
            request: msg.data,
            card_address: self.card_address.clone(),
        };

        if let Some(mut writer) = self.tcp_writer.take() {
            let fut = async move {
                let result =
                    TCPProtocol::send_with_writer(&mut writer, MsgType::FuelRequest, &fuel_request)
                        .await;
                (writer, result)
            }
            .into_actor(self)
            .map(|(writer, result), actor, _ctx| {
                actor.tcp_writer = Some(writer);
                match result {
                    Ok(()) => {
                        debug!("FuelRequest sent to gas pump");
                    }
                    Err(e) => {
                        debug!("Failed to send FuelRequest to gas pump: {}", e);
                    }
                }
            });

            ctx.wait(fut);
        } else {
            debug!("TCP writer not available");
        }
    }
}

impl Handler<FuelComplete> for Card {
    type Result = ();

    fn handle(&mut self, msg: FuelComplete, _ctx: &mut Self::Context) -> Self::Result {
        if let Err(e) = self.fuel_complete_sender.send(msg) {
            debug!("Failed to send FuelComplete to CardClient: {}", e);
        } else {
            debug!("Sent FuelComplete to CardClient via mpsc channel");
        }
    }
}
