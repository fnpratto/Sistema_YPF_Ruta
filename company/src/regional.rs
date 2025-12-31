/*
AdminRegionalHandler

Representa un admin regional. Cuando recibe un pedido de carga:

1) Lee el company_id del empleado.

2) Envía mensaje al RegistryActor:
“Dame el Addr<CompanyHandler> de la empresa X”.

3) Una vez recibido el Addr<CompanyHandler>, le puede enviar un mensaje del estilo:
ProcessFuelRequest { empleado_id, monto, estación_id }.
*/

use actix::dev::ContextFutureSpawner;
use actix::{
    Actor, ActorContext, ActorFutureExt, Addr, AsyncContext, Context, Handler, Message, WrapFuture,
};
use common::{regional_admin_debug, regional_admin_error, regional_admin_info};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
#[allow(unused_imports)]
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::company::{PaymentProcessedAck, ProcessPumpExpense};
use crate::persistence::*;
#[allow(unused_imports)]
use crate::registry::{GetCompanyAddr, RegistryActor};
use common::message::{CompanyMessage, Expense, MsgType, RegionalAdminMessage};
use common::tcp_protocol::TCPProtocol;

#[derive(Message)]
#[rtype(result = "()")]
pub struct ForwardPumpExpense {
    pub pump_address: String,
    pub expense: Expense,
    pub regional_admin_id: String,
    pub service_station_id: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SendPaymentProcessed {
    pub pump_address: String,
    pub expense: Expense,
    pub accepted: bool,
    pub service_station_id: String,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ForwardPaymentAck {
    pub pump_address: String,
    pub expense: Expense,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ServiceStationReconnected {
    pub service_station_id: String,
    pub regional_admin_id: String,
}

#[allow(dead_code)]
pub struct RegionalAdminHandler {
    reg_admin_id: String,
    writer: Option<OwnedWriteHalf>,
    reader: Option<OwnedReadHalf>,
    registry: Addr<RegistryActor>,
    persistence: Addr<PersistenceActor>,
    reader_task_cancel_token: Option<CancellationToken>,
}

impl RegionalAdminHandler {
    pub fn new(
        stream: TcpStream,
        _ctx: &mut Context<Self>,
        reg_admin_id: String,
        registry: Addr<RegistryActor>,
        persistence: Addr<PersistenceActor>,
    ) -> Self {
        let (read, write) = stream.into_split();

        regional_admin_info!(
            "[RegionalAdminHandler {}] AdminRegional connected!",
            reg_admin_id
        );
        RegionalAdminHandler {
            reg_admin_id,
            writer: Some(write),
            reader: Some(read),
            registry,
            persistence,
            reader_task_cancel_token: None,
        }
    }

    fn spawn_read_loop(&mut self, ctx: &mut Context<Self>, cancel_token: CancellationToken) {
        let mut reader = self.reader.take().expect("reader missing");
        let addr = ctx.address();
        let reg_admin_id = self.reg_admin_id.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        regional_admin_debug!("RegionalAdminHandler reader task cancelled gracefully");
                        break;
                    }

                    msg_type_result = TCPProtocol::receive_msg_type_from_reader(&mut reader) => {
                        match msg_type_result {
                            Ok(msg_type) => match msg_type {
                                MsgType::PumpExpense => {
                                    match TCPProtocol::receive_msg_from_reader(&mut reader).await {
                                        Ok(RegionalAdminMessage::PumpExpense { pump_address, expense, service_station_id }) => {
                                            addr.do_send(ForwardPumpExpense {
                                                pump_address,
                                                expense,
                                                regional_admin_id: reg_admin_id.clone(),
                                                service_station_id,
                                            });
                                        }
                                        _ => {
                                            regional_admin_debug!("Unexpected message while parsing PumpExpense");
                                        }
                                    }
                                }
                                MsgType::PaymentProcessedAck => {
                                    match TCPProtocol::receive_msg_from_reader(&mut reader).await {
                                        Ok(RegionalAdminMessage::PaymentProcessedAck { pump_address, expense }) => {
                                            addr.do_send(ForwardPaymentAck { pump_address, expense });
                                        }
                                        _ => {
                                            regional_admin_debug!("Unexpected message while parsing PaymentProcessedAck");
                                        }
                                    }
                                }
                                MsgType::ServiceStationReconnected => {
                                    match TCPProtocol::receive_msg_from_reader(&mut reader).await {
                                        Ok(RegionalAdminMessage::ServiceStationReconnected { service_station_id, regional_admin_id }) => {
                                            regional_admin_info!(
                                                "[RegionalAdminHandler {}] Service station '{}' reconnected",
                                                reg_admin_id,
                                                service_station_id
                                            );
                                            addr.do_send(ServiceStationReconnected {
                                                service_station_id,
                                                regional_admin_id,
                                            });
                                        }
                                        _ => {
                                            regional_admin_debug!("Unexpected message while parsing ServiceStationReconnected");
                                        }
                                    }
                                }
                                _ => {
                                    regional_admin_debug!("Unexpected MsgType received: {:?}", msg_type);
                                }
                            },
                            Err(_) => {
                                regional_admin_debug!("Connection to RegionalAdmin closed");
                                break;
                            }
                        }
                    }
                }
            }
            regional_admin_debug!("RegionalAdminHandler TCP reader task terminated");
        });
    }

    pub fn send_msg<T: serde::Serialize + 'static>(
        &mut self,
        ctx: &mut Context<Self>,
        msg_type: MsgType,
        payload: T,
    ) {
        if let Some(mut writer) = self.writer.take() {
            let fut = async move {
                let res = TCPProtocol::send_with_writer(&mut writer, msg_type, &payload).await;
                (writer, res)
            }
            .into_actor(self)
            .map(|(writer, res), actor, _ctx| {
                actor.writer = Some(writer);
                if let Err(e) = res {
                    regional_admin_error!("send error: {}", e);
                }
            });

            ctx.spawn(fut);
        }
    }
}

impl Actor for RegionalAdminHandler {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        regional_admin_info!(
            "[RegionalAdminHandler {}] Actor started.",
            self.reg_admin_id
        );

        let cancel_token = CancellationToken::new();
        self.reader_task_cancel_token = Some(cancel_token.clone());
        self.spawn_read_loop(ctx, cancel_token);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> actix::Running {
        if let Some(token) = &self.reader_task_cancel_token {
            token.cancel();
            regional_admin_debug!("Cancelling RegionalHandler reader task...");
        }
        actix::Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        regional_admin_info!(
            "[RegionalAdminHandler {}] Actor stopped!",
            self.reg_admin_id,
        );
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect;

impl Handler<Disconnect> for RegionalAdminHandler {
    type Result = ();

    fn handle(&mut self, _msg: Disconnect, ctx: &mut Self::Context) {
        regional_admin_debug!(
            "[RegionalAdminHandler {}] RegionalAdmin disconnected! Cerrando actor...",
            self.reg_admin_id
        );
        ctx.stop();
    }
}

impl Handler<ForwardPumpExpense> for RegionalAdminHandler {
    type Result = ();

    fn handle(&mut self, msg: ForwardPumpExpense, ctx: &mut Self::Context) {
        let company_id = msg.expense.company_id;
        let registry = self.registry.clone();
        let expense_copy = msg.expense.clone();
        let expense = msg.expense;
        let pump_address = msg.pump_address.clone();
        let pump_address_for_response = msg.pump_address;
        let regional_admin_id = msg.regional_admin_id.clone();
        let service_station_id = msg.service_station_id.clone();
        let service_station_id_for_response = msg.service_station_id;
        let self_addr = ctx.address();

        // 1) Pedir CompanyHandler al registry
        async move {
            let maybe_company_addr = registry
                .send(GetCompanyAddr { company_id })
                .await
                .unwrap_or(None);

            (
                maybe_company_addr,
                expense,
                pump_address,
                regional_admin_id,
                service_station_id,
                self_addr,
            )
        }
        .into_actor(self)
        .then(
            |(
                maybe_company_addr,
                expense,
                pump_address,
                regional_admin_id,
                service_station_id,
                self_addr,
            ),
             actor,
             ctx| {
                if let Some(company_addr) = maybe_company_addr {
                    // Store this handler in the CompanyHandler for future responses
                    company_addr.do_send(crate::company::StoreRegionalAdminHandler {
                        regional_admin_id: regional_admin_id.clone(),
                        handler: self_addr,
                    });

                    // 2) Forwardear gasto al CompanyHandler
                    async move {
                        let result = company_addr
                            .send(ProcessPumpExpense {
                                pump_address: pump_address.clone(),
                                expense: expense_copy,
                                regional_admin_id: regional_admin_id.clone(),
                                service_station_id: service_station_id.clone(),
                            })
                            .await
                            .unwrap_or(false);
                        (result, pump_address, expense)
                    }
                    .into_actor(actor)
                    .map(|(response, pump_address, expense), actor, ctx| {
                        // 3) Enviar respuesta al RegionalAdmin
                        actor.send_msg(
                            ctx,
                            MsgType::PaymentProcessed,
                            CompanyMessage::PaymentProcessed {
                                pump_address,
                                expense,
                                accepted: response,
                                service_station_id: service_station_id_for_response,
                            },
                        );
                    })
                    .spawn(ctx);
                } else {
                    // Empresa no encontrada
                    let fail = CompanyMessage::PaymentProcessed {
                        pump_address: pump_address_for_response,
                        expense,
                        accepted: false,
                        service_station_id: service_station_id_for_response,
                    };
                    actor.send_msg(ctx, MsgType::PaymentProcessed, fail);
                }

                async {}.into_actor(actor)
            },
        )
        .spawn(ctx);
    }
}

impl Handler<ForwardPaymentAck> for RegionalAdminHandler {
    type Result = ();

    fn handle(&mut self, msg: ForwardPaymentAck, ctx: &mut Self::Context) {
        let company_id = msg.expense.company_id;
        let registry = self.registry.clone();
        let expense = msg.expense.clone();
        let pump_address = msg.pump_address.clone();

        // Forward ACK to the appropriate CompanyHandler
        async move {
            let addr = registry
                .send(GetCompanyAddr { company_id })
                .await
                .unwrap_or(None);
            (addr, company_id, pump_address, expense)
        }
        .into_actor(self)
        .then(
            |(maybe_company_addr, company_id, pump_address, expense), _actor, _ctx| {
                if let Some(company_addr) = maybe_company_addr {
                    regional_admin_debug!(
                        "Forwarding PaymentAck to Company {} for pump '{}'",
                        company_id,
                        pump_address
                    );
                    company_addr.do_send(PaymentProcessedAck {
                        pump_address,
                        expense,
                    });
                } else {
                    regional_admin_error!(
                        "Cannot forward PaymentProcessedAck: Company {} not found",
                        company_id
                    );
                }
                async {}.into_actor(_actor)
            },
        )
        .spawn(ctx);
    }
}

impl Handler<SendPaymentProcessed> for RegionalAdminHandler {
    type Result = ();

    fn handle(&mut self, msg: SendPaymentProcessed, ctx: &mut Self::Context) {
        regional_admin_debug!(
            "[RegionalAdminHandler {}] Sending PaymentProcessed for pump '{}': accepted={}, total={}",
            self.reg_admin_id,
            msg.pump_address,
            msg.accepted,
            msg.expense.total
        );

        let payment_msg = CompanyMessage::PaymentProcessed {
            pump_address: msg.pump_address,
            expense: msg.expense,
            accepted: msg.accepted,
            service_station_id: msg.service_station_id,
        };

        self.send_msg(ctx, MsgType::PaymentProcessed, payment_msg);
    }
}

impl Handler<ServiceStationReconnected> for RegionalAdminHandler {
    type Result = ();

    fn handle(&mut self, msg: ServiceStationReconnected, ctx: &mut Self::Context) {
        regional_admin_info!(
            "[RegionalAdminHandler {}] Service station '{}' reconnected via regional_admin '{}'",
            self.reg_admin_id,
            msg.service_station_id,
            msg.regional_admin_id
        );

        let registry = self.registry.clone();
        let regional_admin_id = msg.regional_admin_id.clone();
        let self_addr = ctx.address();
        let service_station_id = msg.service_station_id.clone();

        async move { (registry, regional_admin_id, self_addr, service_station_id) }
            .into_actor(self)
            .then(
                |(registry, regional_admin_id, self_addr, service_station_id), _actor, _ctx| {
                    registry.do_send(crate::registry::NotifyServiceStationReconnection {
                        service_station_id: service_station_id.clone(),
                        regional_admin_id: regional_admin_id.clone(),
                        regional_admin_handler: self_addr,
                    });

                    async {}.into_actor(_actor)
                },
            )
            .spawn(ctx);
    }
}
