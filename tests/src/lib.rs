use log::info;
use std::time::Duration;
use tokio::time::sleep;

// Import the colored macros from common
use common::{crash_info, election_debug, election_info, fuel_debug, fuel_info, leader_info};
pub mod communication_test;

/// Constants for testing
pub const PUMP_BASE_PORT: u32 = 9000;
pub const CARD_BASE_PORT: u32 = 8000;
pub const COMPANY_BASE_PORT: u32 = 7000;
pub const PRICE_PER_LITER: f64 = 1.5;

/// Test Network Simulator
pub struct TestNetwork {
    pub pump_count: u32,
    pub card_count: u32,
    pub company_count: u32,
}

impl TestNetwork {
    /// Create a new test network
    pub async fn new(pump_count: u32, card_count: u32, company_count: u32) -> Self {
        info!("Creating test network:");
        info!("   {} gas pumps", pump_count);
        info!("   {} cards", card_count);
        info!("   {} companies", company_count);

        // Simulate initialization time
        sleep(Duration::from_millis(100 * pump_count as u64)).await;

        info!("Test network created successfully");

        Self {
            pump_count,
            card_count,
            company_count,
        }
    }

    /// Get the expected leader (highest port pump)
    pub fn get_expected_leader(&self) -> u32 {
        PUMP_BASE_PORT + self.pump_count - 1
    }

    /// Simulate leader election with colored output
    pub async fn simulate_leader_election(&self) {
        election_info!("Starting leader election simulation");
        election_info!("   {} pumps participating", self.pump_count);

        for round in 1..=3 {
            election_debug!("   Election round {}/3", round);
            sleep(Duration::from_millis(100)).await;
        }

        let leader = self.get_expected_leader();
        leader_info!("Leader elected: Pump {} (highest ID)", leader);
    }

    /// Simulate leader crash and recovery with dramatic colors
    pub async fn simulate_leader_crash(&self) {
        let leader = self.get_expected_leader();
        crash_info!("Simulating crash of leader pump {}", leader);

        sleep(Duration::from_millis(200)).await;
        crash_info!("   Crash detected by other pumps");

        sleep(Duration::from_millis(300)).await;
        election_info!("   New election triggered");

        self.simulate_leader_election().await;
        leader_info!("System recovered from crash");
    }

    /// Simulate expense transaction with fuel colors
    pub async fn simulate_transaction(&self, card_id: u32, pump_id: u32, liters: f64) -> bool {
        let pump_port = PUMP_BASE_PORT + pump_id;
        let total = liters * PRICE_PER_LITER;

        fuel_info!(
            "Transaction: Card {} → Pump {} ({} liters, ${})",
            card_id,
            pump_port,
            liters,
            total
        );

        // Simulate transaction processing
        sleep(Duration::from_millis(150)).await;

        fuel_debug!("   Transaction completed");
        true
    }

    /// Simulate multiple transactions with stress indicators
    pub async fn stress_test_transactions(&self, count: u32) {
        crash_info!("Starting stress test with {} transactions", count);

        for i in 0..count {
            let card_id = i % self.card_count.max(1);
            let pump_id = i % self.pump_count;
            let liters = 15.0 + (i as f64 * 5.0);

            fuel_debug!("Stress transaction {} of {}", i + 1, count);
            self.simulate_transaction(card_id, pump_id, liters).await;

            // Quick pause between stress transactions
            sleep(Duration::from_millis(25)).await;
        }

        crash_info!("Stress test completed - {} transactions processed", count);
    }

    /// System health check with colored status
    pub async fn verify_system_health(&self) {
        info!("System Health Check:");
        leader_info!("   Pumps: {} active", self.pump_count);
        fuel_info!("   Cards: {} active", self.card_count);
        election_info!("   Companies: {} active", self.company_count);

        let expected_leader = self.get_expected_leader();
        leader_info!("   Expected leader: pump {}", expected_leader);

        info!("System health verified");
    }

    /// Wait for specified time
    pub async fn wait(&self, millis: u64) {
        sleep(Duration::from_millis(millis)).await;
    }
}

// Include test modules
pub mod bully_tests;
pub mod expense_tests;
pub mod integration_tests;
pub mod pending_results_test;
pub mod performance_tests;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
