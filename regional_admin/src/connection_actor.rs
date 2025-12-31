use actix::prelude::*;
use common::message::{CompanyMessage, MsgType, RegionalAdminMessage};
use common::tcp_protocol::TCPProtocol;
use common::{crash_info, election_debug, fuel_debug, fuel_info, leader_debug};
use log::{debug, error};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::message::SendToCompanyFailed;
use crate::message::*;
use crate::regional_admin::RegionalAdmin;

pub struct RegionalAdminConnectionActor {
    admin_addr: Addr<RegionalAdmin>,
    writer: Option<OwnedWriteHalf>,
}

impl RegionalAdminConnectionActor {
    pub fn start(
        reader: OwnedReadHalf,
        writer: OwnedWriteHalf,
        admin_addr: Addr<RegionalAdmin>,
        peer_address: String,
    ) -> Addr<Self> {
        fuel_debug!("📡 Starting ConnectionActor for peer: {}", peer_address);

        Self::create(|ctx| {
            let conn_addr = ctx.address();
            Self::spawn_reader_task(reader, admin_addr.clone(), peer_address.clone(), conn_addr);

            Self {
                admin_addr: admin_addr.clone(),
                writer: Some(writer),
            }
        })
    }

    fn spawn_reader_task(
        mut reader: OwnedReadHalf,
        admin_addr: Addr<RegionalAdmin>,
        peer_address: String,
        conn_addr: Addr<RegionalAdminConnectionActor>,
    ) {
        fuel_debug!("🔄 Starting reader task for peer: {}", peer_address);

        actix::spawn(async move {
            loop {
                tokio::select! {
                    msg_type_result = TCPProtocol::receive_msg_type_from_reader(&mut reader) => {
                        match msg_type_result {
                            Ok(msg_type) => {
                                let should_break = match msg_type {
                                    MsgType::PumpExpense => {
                                        Self::handle_pump_expense(&mut reader, &admin_addr, &conn_addr, &peer_address).await
                                    },
                                    MsgType::PaymentProcessed => {
                                        Self::handle_payment_processed(&mut reader, &admin_addr, &peer_address).await
                                    },
                                    _ => {
                                        election_debug!("❓ ConnectionActor unexpected MsgType from {}: {:?}", peer_address, msg_type);
                                        false
                                    }
                                };
                                if should_break {
                                    break;
                                }
                            },
                            Err(_) => {
                                fuel_debug!("🔌 Connection to {} closed", peer_address);
                                break;
                            }
                        }
                    }
                }
            }
            leader_debug!(
                "🛑 ConnectionActor TCP reader task terminated for {}",
                peer_address
            );
        });
    }

    async fn handle_pump_expense(
        reader: &mut OwnedReadHalf,
        admin_addr: &Addr<RegionalAdmin>,
        conn_addr: &Addr<RegionalAdminConnectionActor>,
        peer_address: &str,
    ) -> bool {
        match TCPProtocol::receive_msg_from_reader::<RegionalAdminMessage>(reader).await {
            Ok(RegionalAdminMessage::PumpExpense {
                pump_address,
                expense,
                service_station_id,
            }) => {
                fuel_info!(
                    "💰 Received PumpExpense from {}: company_id={}, total=${}, service_station={}",
                    pump_address,
                    expense.company_id,
                    expense.total,
                    service_station_id
                );
                let pump_expense = PumpExpense::with_connection(
                    pump_address.clone(),
                    expense,
                    service_station_id,
                    conn_addr.clone(),
                );

                admin_addr.do_send(pump_expense);
                false
            }
            Ok(_) => {
                crash_info!(
                    "❌ Unexpected message while parsing PumpExpense from {}",
                    peer_address
                );
                true
            }
            Err(e) => {
                crash_info!("❌ Error parsing PumpExpense from {}: {}", peer_address, e);
                true
            }
        }
    }

    async fn handle_payment_processed(
        reader: &mut OwnedReadHalf,
        admin_addr: &Addr<RegionalAdmin>,
        peer_address: &str,
    ) -> bool {
        match TCPProtocol::receive_msg_from_reader::<CompanyMessage>(reader).await {
            Ok(CompanyMessage::PaymentProcessed {
                pump_address,
                expense,
                accepted,
                service_station_id,
            }) => {
                if accepted {
                    fuel_info!(
                        "✅ Received PaymentProcessed ACCEPTED from {}: pump={}, total=${}, service_station={}",
                        peer_address,
                        pump_address,
                        expense.total,
                        service_station_id
                    );
                } else {
                    crash_info!(
                        "❌ Received PaymentProcessed REJECTED from {}: pump={}, total=${}, service_station={}",
                        peer_address,
                        pump_address,
                        expense.total,
                        service_station_id
                    );
                }

                let payment_processed = CompanyPaymentProcessed::new(
                    pump_address.clone(),
                    expense,
                    accepted,
                    service_station_id,
                );
                admin_addr.do_send(payment_processed);
                false
            }
            Err(e) => {
                crash_info!(
                    "❌ Error parsing PaymentProcessed from {}: {}",
                    peer_address,
                    e
                );
                true
            }
        }
    }
}

fn process_result<T, E: std::fmt::Debug>(result: Result<T, E>, msg_type: MsgType) {
    match result {
        Ok(_) => fuel_debug!("✅ Sent message of type {:?}", msg_type),
        Err(e) => error!("❌ Failed to send message {:?}: {:?}", msg_type, e),
    }
}

impl Actor for RegionalAdminConnectionActor {
    type Context = Context<Self>;
}

impl Handler<SendToClient<RegionalAdminMessage>> for RegionalAdminConnectionActor {
    type Result = ();

    fn handle(
        &mut self,
        msg: SendToClient<RegionalAdminMessage>,
        ctx: &mut Self::Context,
    ) -> Self::Result {
        fuel_debug!("📤 ConnectionActor sending {:?} message", msg.msg_type);

        if let Some(mut writer) = self.writer.take() {
            let msg_type = msg.msg_type.clone();
            let message_clone = msg.msg.clone();
            let fut = async move {
                let send_result =
                    TCPProtocol::send_with_writer(&mut writer, msg_type.clone(), &message_clone)
                        .await;
                let is_ok = send_result.is_ok();
                process_result(send_result, msg_type.clone());
                (writer, is_ok)
            }
            .into_actor(self)
            .map(|(writer, is_ok), actor, _ctx| {
                actor.writer = Some(writer);

                if !is_ok {
                    crash_info!(
                        "❌ ConnectionActor failed to send message - connection may be broken"
                    );

                    actor
                        .admin_addr
                        .do_send(SendToCompanyFailed { message: msg.msg });
                }
            });

            ctx.wait(fut);
        } else {
            crash_info!("❌ ConnectionActor TCP writer not available - cannot send message");
        }
    }
}

impl Handler<SendPaymentProcessedToPump> for RegionalAdminConnectionActor {
    type Result = ();

    fn handle(&mut self, msg: SendPaymentProcessedToPump, ctx: &mut Self::Context) -> Self::Result {
        fuel_debug!(
            "📤 ConnectionActor sending PaymentProcessed to pump '{}'",
            msg.pump_address
        );

        if let Some(mut writer) = self.writer.take() {
            let pump_address = msg.pump_address.clone();
            let expense = msg.expense.clone();
            let accepted = msg.accepted;
            let service_station_id = msg.service_station_id.clone();
            let admin_addr = self.admin_addr.clone();

            let response = CompanyMessage::PaymentProcessed {
                pump_address: pump_address.clone(),
                expense: expense.clone(),
                accepted,
                service_station_id: service_station_id.clone(),
            };

            let fut = async move {
                let send_result =
                    TCPProtocol::send_with_writer(&mut writer, MsgType::PaymentProcessed, &response).await;
                (writer, send_result, pump_address, expense, accepted, service_station_id, admin_addr)
            }
            .into_actor(self)
            .map(|(writer, send_result, pump_address, expense, accepted, service_station_id, admin_addr), actor, _ctx| {
                actor.writer = Some(writer);

                match send_result {
                    Ok(_) => {
                        fuel_debug!(
                            "✅ ConnectionActor PaymentProcessed sent successfully to pump '{}'",
                            pump_address
                        );

                        admin_addr.do_send(SendToPumpSuccess {
                            pump_address: pump_address.clone(),
                            expense: expense.clone(), // TODO: ID for each message
                        });
                    }
                    Err(_) => {

                        admin_addr.do_send(SendToPumpFailed {
                            pump_address,
                            service_station_id,
                            msg_type: MsgType::PaymentProcessed,
                            expense,
                            accepted,
                        });
                    }
                }
            });

            ctx.wait(fut);
        } else {
            crash_info!(
                "❌ ConnectionActor TCP writer not available for service_station '{}' - notifying to buffer",
                msg.service_station_id
            );

            self.admin_addr.do_send(SendToPumpFailed {
                pump_address: msg.pump_address,
                service_station_id: msg.service_station_id,
                msg_type: MsgType::PaymentProcessed,
                expense: msg.expense,
                accepted: msg.accepted,
            });
        }
    }
}
