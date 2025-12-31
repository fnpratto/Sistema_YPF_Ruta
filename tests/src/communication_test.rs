#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use tokio::sync::{mpsc, Mutex};
#[allow(unused_imports)]
use tokio::time::timeout;

// Import all actors and their messages
#[allow(unused_imports)]
use card::message::{FuelComplete, FuelRequest as CardFuelRequest};
#[allow(unused_imports)]
use company::company::CompanyHandler;
#[allow(unused_imports)]
use company::persistence::PersistenceActor;
#[allow(unused_imports)]
use company::registry::RegistryActor;
#[allow(unused_imports)]
use gas_pump::message::{GetLeaderInfo, UpdatePumps};
#[allow(unused_imports)]
use gas_pump::pump::Pump;
#[allow(unused_imports)]
use regional_admin::regional_admin::RegionalAdmin;

#[allow(unused_imports)]
use common::message::{Expense, FuelRequestData, Start};

#[allow(unused_imports)]
use crate::*;
#[allow(unused_imports)]
use common::{crash_info, election_debug, election_info, fuel_debug, fuel_info, leader_info};

#[tokio::test]
async fn test_card_pump_communication_only() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    fuel_info!("TEST: Card-Pump Communication");

    let network = TestNetwork::new(3, 2, 1).await;

    // Setup network with leader election
    network.simulate_leader_election().await;
    network.wait(500).await;

    // Simulate card-pump communication
    fuel_info!("Simulating card transactions...");
    for i in 0..3 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let liters = 25.0 + i as f64 * 5.0;

        let success = network.simulate_transaction(card_id, pump_id, liters).await;
        assert!(success, "Transaction should succeed");
        fuel_debug!("Transaction {} completed successfully", i + 1);
    }

    network.verify_system_health().await;
    fuel_info!("Card-Pump communication test completed successfully");
}

#[tokio::test]
async fn test_regional_admin_pump_connection() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    election_info!("TEST: Regional Admin-Pump Connection");

    let network = TestNetwork::new(4, 1, 2).await;

    // Setup with regional admin coordination
    election_info!("Setting up regional admin network...");
    network.simulate_leader_election().await;
    network.wait(400).await;

    // Test pump registration with regional admin
    election_debug!("Testing pump registration...");
    network.wait(300).await;

    // Simulate some transactions to test the connection
    fuel_info!("Testing transactions through regional admin...");
    for i in 0..2 {
        let success = network
            .simulate_transaction(0, i % network.pump_count, 30.0)
            .await;
        assert!(success, "Regional admin transaction should succeed");
    }

    network.verify_system_health().await;
    election_info!("Regional Admin-Pump connection test completed");
}

#[tokio::test]
async fn test_company_expense_processing() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    fuel_info!("TEST: Company Expense Processing");

    let network = TestNetwork::new(3, 3, 2).await;

    // Setup system
    network.simulate_leader_election().await;
    network.wait(300).await;

    // Process multiple expenses for different companies
    fuel_info!("Processing company expenses...");
    for company_id in 0..network.company_count {
        for card_id in 0..network.card_count {
            let pump_id = (company_id + card_id) % network.pump_count;
            let liters = 20.0 + (company_id + card_id) as f64 * 8.0;

            let success = network.simulate_transaction(card_id, pump_id, liters).await;
            assert!(success, "Company expense processing should succeed");

            fuel_debug!(
                "Expense processed: Company {}, Card {}, {} liters",
                company_id,
                card_id,
                liters
            );
        }
        network.wait(100).await;
    }

    network.verify_system_health().await;
    fuel_info!("Company expense processing test completed");
}

#[tokio::test]
async fn test_pump_leader_communication() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    election_info!("TEST: Multi-Pump Network Communication");

    let network = TestNetwork::new(6, 4, 2).await;

    // Test leader election in larger network
    election_info!("Testing leader election in 6-pump network...");
    network.simulate_leader_election().await;
    network.wait(600).await;

    let expected_leader = network.get_expected_leader();
    leader_info!("Expected leader in network: pump {}", expected_leader);

    // Test communication across the network
    fuel_info!("Testing cross-network communication...");
    for i in 0..8 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let liters = 15.0 + i as f64 * 4.0;

        let success = network.simulate_transaction(card_id, pump_id, liters).await;
        assert!(success, "Network communication should work");

        if i % 3 == 0 {
            fuel_debug!("Network transaction batch {} completed", i / 3 + 1);
        }

        network.wait(50).await;
    }

    network.verify_system_health().await;
    election_info!("Multi-pump network communication test completed");
}

#[tokio::test]
async fn test_communication_during_leader_change() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    crash_info!("TEST: Communication During Leader Change");

    let network = TestNetwork::new(5, 3, 1).await;

    // Initial setup
    election_info!("Initial leader election...");
    network.simulate_leader_election().await;
    network.wait(400).await;

    // Process some transactions
    fuel_info!("Processing pre-crash transactions...");
    for i in 0..2 {
        let success = network.simulate_transaction(i, i, 25.0).await;
        assert!(success, "Pre-crash transaction should succeed");
    }

    // Simulate leader crash and recovery
    crash_info!("Simulating leader crash during operation...");
    network.simulate_leader_crash().await;

    // Continue processing after recovery
    fuel_info!("Processing post-recovery transactions...");
    for i in 0..3 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let success = network.simulate_transaction(card_id, pump_id, 20.0).await;
        assert!(success, "Post-recovery transaction should succeed");
        network.wait(100).await;
    }

    network.verify_system_health().await;
    leader_info!("Communication during leader change test completed");
}

#[tokio::test]
async fn test_stress_communication() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    crash_info!("TEST: Stress Communication");

    let network = TestNetwork::new(4, 5, 2).await;

    // Setup
    network.simulate_leader_election().await;
    network.wait(300).await;

    // Stress test communication
    fuel_info!("Starting communication stress test...");
    network.stress_test_transactions(20).await;

    // Verify system health after stress
    network.verify_system_health().await;

    crash_info!("Stress communication test completed successfully");
}

#[tokio::test]
async fn test_complete_communication_flow() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    election_info!("TEST: Complete Communication Flow");

    let network = TestNetwork::new(4, 3, 2).await;

    // Phase 1: Network setup
    election_info!("Phase 1: Network initialization");
    network.simulate_leader_election().await;
    network.wait(400).await;

    // Phase 2: Normal operations
    fuel_info!("Phase 2: Normal communication operations");
    for i in 0..5 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let liters = 22.0 + i as f64 * 6.0;

        let success = network.simulate_transaction(card_id, pump_id, liters).await;
        assert!(success, "Normal operation should succeed");
        network.wait(80).await;
    }

    // Phase 3: System under stress
    crash_info!("Phase 3: Communication under stress");
    network.stress_test_transactions(8).await;

    // Phase 4: Recovery verification
    leader_info!("Phase 4: System recovery verification");
    network.verify_system_health().await;

    election_info!("Complete communication flow test completed successfully");
}
