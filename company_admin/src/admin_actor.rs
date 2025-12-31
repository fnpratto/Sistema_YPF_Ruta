use actix::{Actor, ActorFutureExt, AsyncContext, Context, Handler, Message, System, WrapFuture};
use common::{company_debug, company_error, company_info};
use serde_json::Value;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use common::message::*;
use common::tcp_protocol::TCPProtocol;

#[allow(dead_code)]
pub struct CompanyAdminActor {
    reader: Option<OwnedReadHalf>,
    writer: Option<OwnedWriteHalf>,
    company_id: u32,
    sender: mpsc::UnboundedSender<String>,
    reader_task_cancel_token: Option<CancellationToken>,
}

impl CompanyAdminActor {
    pub fn new(
        writer: OwnedWriteHalf,
        reader: OwnedReadHalf,
        company_id: u32,
        sender: mpsc::UnboundedSender<String>,
    ) -> Self {
        Self {
            reader: Some(reader),
            writer: Some(writer),
            company_id,
            sender,
            reader_task_cancel_token: None,
        }
    }

    fn spawn_reader_loop(
        &self,
        mut reader: tokio::net::tcp::OwnedReadHalf,
        addr: actix::Addr<CompanyAdminActor>,
        cancel_token: CancellationToken,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        company_debug!("CompanyAdminActor reader task cancelled gracefully");
                        break;
                    }

                    msg_type_result = TCPProtocol::receive_msg_type_from_reader(&mut reader) => {
                        match msg_type_result {
                            Ok(msg_type) => {
                                let payload: Value = match TCPProtocol::receive_msg_from_reader(&mut reader).await {
                                    Ok(p) => p,
                                    Err(_) => break,
                                };
                                addr.do_send(TcpIncoming { msg_type, payload });
                            },
                            Err(_) => {
                                company_debug!("Connection to server closed");
                                company_debug!("Terminating program - cannot proceed without server connection");
                                System::current().stop();
                                std::process::exit(1);
                            }
                        }
                    }
                }
            }
            company_debug!("CompanyAdmin TCP reader task terminated");
        });
    }

    fn send_json(&mut self, ctx: &mut Context<Self>, msg_type: MsgType, payload: Value) {
        if let Some(mut writer) = self.writer.take() {
            let fut = async move {
                let r = TCPProtocol::send_with_writer(&mut writer, msg_type, &payload).await;
                (writer, r)
            }
            .into_actor(self)
            .map(|(writer, r), actor, _| {
                actor.writer = Some(writer);
                if let Err(e) = r {
                    company_error!("send error: {}", e);
                }
            });

            ctx.spawn(fut);
        }
    }
}

impl Actor for CompanyAdminActor {
    type Context = Context<Self>;

    fn stopping(&mut self, _ctx: &mut Self::Context) -> actix::Running {
        if let Some(token) = &self.reader_task_cancel_token {
            token.cancel();
            company_debug!("Cancelling CompanyAdmin reader task...");
        }
        actix::Running::Stop
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        company_info!("[CompanyAdmin {}] Actor stopped!", self.company_id);
    }

    fn started(&mut self, ctx: &mut Context<Self>) {
        company_info!("[CompanyAdmin {}] Actor started!", self.company_id);
        let reader = self.reader.take().unwrap();

        let cancel_token = CancellationToken::new();
        self.reader_task_cancel_token = Some(cancel_token.clone());

        let addr = ctx.address();

        self.spawn_reader_loop(reader, addr, cancel_token);
    }
}

#[allow(dead_code)]
pub struct TcpIncoming {
    pub msg_type: MsgType,
    pub payload: serde_json::Value,
}

impl Message for TcpIncoming {
    type Result = ();
}

impl Handler<TcpIncoming> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, incoming: TcpIncoming, _: &mut Context<Self>) {
        let _ = self.sender.send(incoming.payload.to_string());
    }
}

impl Handler<AskTotalBalance> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, _msg: AskTotalBalance, ctx: &mut Self::Context) {
        self.send_json(ctx, MsgType::AskTotalBalance, serde_json::json!({}));
    }
}

impl Handler<ChargeBalance> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, msg: ChargeBalance, ctx: &mut Self::Context) {
        self.send_json(
            ctx,
            MsgType::ChargeBalance,
            serde_json::json!({ "amount": msg.amount }),
        );
    }
}

impl Handler<RegisterNewCard> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, msg: RegisterNewCard, ctx: &mut Self::Context) {
        self.send_json(
            ctx,
            MsgType::RegisterNewCard,
            serde_json::json!({ "card_id": msg.card_id, "amount": msg.amount }),
        );
    }
}

impl Handler<CheckCardLimit> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, msg: CheckCardLimit, ctx: &mut Self::Context) {
        self.send_json(
            ctx,
            MsgType::CheckCardLimit,
            serde_json::json!({ "card_id": msg.card_id }),
        );
    }
}

impl Handler<EstablishCardBalance> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, msg: EstablishCardBalance, ctx: &mut Self::Context) {
        self.send_json(
            ctx,
            MsgType::EstablishCardBalance,
            serde_json::json!({ "card_id": msg.card_id, "amount": msg.amount }),
        );
    }
}

impl Handler<CheckAllCards> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, _msg: CheckAllCards, ctx: &mut Self::Context) {
        self.send_json(ctx, MsgType::CheckAllCards, serde_json::json!({}));
    }
}

impl Handler<CheckExpenses> for CompanyAdminActor {
    type Result = ();

    fn handle(&mut self, _msg: CheckExpenses, ctx: &mut Self::Context) {
        self.send_json(ctx, MsgType::CheckExpenses, serde_json::json!({}));
    }
}
