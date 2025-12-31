use actix::prelude::*;
use common::{company_debug, company_info};
use std::collections::HashMap;

use crate::company::CompanyHandler;
use crate::company::{AttachCompanyAdmin, ServiceStationReconnected, StoreRegionalAdminHandler};
use crate::persistence::PersistenceActor;
use crate::regional::RegionalAdminHandler;
use tokio::net::TcpStream;

pub type CompanyId = u32;

pub struct RegistryActor {
    companies: HashMap<CompanyId, Addr<CompanyHandler>>,
    persistence: Addr<PersistenceActor>,
}

impl RegistryActor {
    pub fn new(persistence: Addr<PersistenceActor>) -> Self {
        RegistryActor {
            companies: HashMap::new(),
            persistence,
        }
    }
}

impl Actor for RegistryActor {
    type Context = Context<Self>;
}

/// Message for obtain CompanyHandler address
#[derive(Message)]
#[rtype(result = "Option<Addr<CompanyHandler>>")]
pub struct GetCompanyAddr {
    pub company_id: CompanyId,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct CreateOrConnectCompany {
    pub company_id: CompanyId,
    pub admin_stream: Option<TcpStream>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct NotifyServiceStationReconnection {
    pub service_station_id: String,
    pub regional_admin_id: String,
    pub regional_admin_handler: Addr<RegionalAdminHandler>,
}

impl Handler<GetCompanyAddr> for RegistryActor {
    type Result = MessageResult<GetCompanyAddr>;

    fn handle(&mut self, msg: GetCompanyAddr, _ctx: &mut Self::Context) -> Self::Result {
        let res = self.companies.get(&msg.company_id).cloned();
        MessageResult(res)
    }
}

impl Handler<CreateOrConnectCompany> for RegistryActor {
    type Result = ();

    fn handle(&mut self, msg: CreateOrConnectCompany, _ctx: &mut Context<Self>) {
        let company_id = msg.company_id;

        if let Some(addr) = self.companies.get(&company_id) {
            company_debug!(
                "[REGISTRY] CompanyHandler {} already exists! Connecting CompanyAdmin...",
                company_id
            );

            if let Some(stream) = msg.admin_stream {
                addr.do_send(AttachCompanyAdmin { stream });
            }
            return;
        }

        let persistence = self.persistence.clone();
        let addr = CompanyHandler::create(|_| CompanyHandler::new(company_id, persistence));
        if let Some(stream) = msg.admin_stream {
            addr.do_send(AttachCompanyAdmin { stream });
        }

        self.companies.insert(company_id, addr);
        company_info!("[REGISTRY] CompanyHandler {} created!", company_id);
    }
}

impl Handler<NotifyServiceStationReconnection> for RegistryActor {
    type Result = ();

    fn handle(&mut self, msg: NotifyServiceStationReconnection, _ctx: &mut Self::Context) {
        company_info!(
            "[REGISTRY] Notifying all companies about service_station '{}' reconnection via regional_admin '{}'",
            msg.service_station_id,
            msg.regional_admin_id
        );

        for (company_id, company_addr) in self.companies.iter() {
            company_addr.do_send(StoreRegionalAdminHandler {
                regional_admin_id: msg.regional_admin_id.clone(),
                handler: msg.regional_admin_handler.clone(),
            });

            company_addr.do_send(ServiceStationReconnected {
                service_station_id: msg.service_station_id.clone(),
                regional_admin_id: msg.regional_admin_id.clone(),
            });
            company_info!(
                "[REGISTRY] Notified company {} about service_station '{}' reconnection",
                company_id,
                msg.service_station_id
            );
        }
    }
}
