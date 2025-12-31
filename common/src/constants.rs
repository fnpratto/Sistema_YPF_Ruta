#![allow(dead_code)]
pub const ADDRESS_COMPANY: &str = "127.0.0.1:8080";
pub const PUMP_BASE_PORT: u32 = 10000;
pub const ADDRESS_PUMP_BASE: &str = "127.0.0.1:";
pub const PRICEPERLITER: u32 = 1606;
pub const ADDRESS_REGIONAL_ADMIN: &str = "127.0.0.1:";
pub const REGIONAL_ADMIN_PORT: u32 = 30000;

pub const LOCALHOST: &str = "127.0.0.1";
pub const COMPANY_LEADER_PORT: u16 = 60000;
pub const COMPANIES_FINAL_PORT: u16 = 60009;

pub const ADDRESS_SERVICE_STATION: &str = "127.0.0.1";

/// Argentine provinces with their corresponding regional admin IDs and addresses
#[derive(Debug, Clone, Copy)]
pub struct Province {
    pub id: u32,
    pub name: &'static str,
    pub address: &'static str,
}

pub const PROVINCES: [Province; 16] = [
    Province {
        id: 0,
        name: "Buenos Aires",
        address: "127.0.0.1:30001",
    },
    Province {
        id: 1,
        name: "Catamarca",
        address: "127.0.0.1:30002",
    },
    Province {
        id: 2,
        name: "Chaco",
        address: "127.0.0.1:30003",
    },
    Province {
        id: 3,
        name: "Chubut",
        address: "127.0.0.1:30004",
    },
    Province {
        id: 4,
        name: "Córdoba",
        address: "127.0.0.1:30005",
    },
    Province {
        id: 5,
        name: "Corrientes",
        address: "127.0.0.1:30006",
    },
    Province {
        id: 6,
        name: "Entre Ríos",
        address: "127.0.0.1:30007",
    },
    Province {
        id: 7,
        name: "Formosa",
        address: "127.0.0.1:30008",
    },
    Province {
        id: 8,
        name: "Jujuy",
        address: "127.0.0.1:30009",
    },
    Province {
        id: 9,
        name: "La Pampa",
        address: "127.0.0.1:30010",
    },
    Province {
        id: 10,
        name: "La Rioja",
        address: "127.0.0.1:30011",
    },
    Province {
        id: 11,
        name: "Mendoza",
        address: "127.0.0.1:30012",
    },
    Province {
        id: 12,
        name: "Misiones",
        address: "127.0.0.1:30013",
    },
    Province {
        id: 13,
        name: "Neuquén",
        address: "127.0.0.1:30014",
    },
    Province {
        id: 14,
        name: "Río Negro",
        address: "127.0.0.1:30015",
    },
    Province {
        id: 15,
        name: "Salta",
        address: "127.0.0.1:30016",
    },
];

/// Get province information by ID
pub fn get_province_by_id(id: u32) -> Option<&'static Province> {
    PROVINCES.iter().find(|province| province.id == id)
}

/// Get regional admin address for a province ID
pub fn get_regional_admin_address(province_id: u32) -> Option<&'static str> {
    get_province_by_id(province_id).map(|province| province.address)
}

/// Get province name by ID
pub fn get_province_name(province_id: u32) -> Option<&'static str> {
    get_province_by_id(province_id).map(|province| province.name)
}

/// Validate province ID
pub fn is_valid_province_id(id: u32) -> bool {
    id < PROVINCES.len() as u32
}
