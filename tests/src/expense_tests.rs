#[allow(unused_imports)]
use crate::*;
#[allow(unused_imports)]
use common::{crash_info, election_info, fuel_info, leader_info};

#[tokio::test]
async fn test_expense_recovery_basic() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    fuel_info!("TEST: Basic Expense Recovery");

    let network = TestNetwork::new(3, 2, 1).await;

    // Initial setup
    network.simulate_leader_election().await;
    network.wait(300).await;

    // Process some expenses
    fuel_info!("Processing test expenses");
    for i in 0..3 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let liters = 20.0 + i as f64 * 10.0;

        network.simulate_transaction(card_id, pump_id, liters).await;
        network.wait(100).await;
    }

    fuel_info!("Expense recovery basic test completed");
}

#[tokio::test]
async fn test_expense_consistency_during_crash() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    crash_info!("TEST: Expense Consistency During Crash");

    let network = TestNetwork::new(5, 3, 2).await;

    // Phase 1: Normal operation
    election_info!("Phase 1: Normal operations");
    network.simulate_leader_election().await;

    for i in 0..2 {
        network
            .simulate_transaction(i, i, 25.0 + i as f64 * 5.0)
            .await;
    }

    // Phase 2: Crash during operations
    crash_info!("Phase 2: Leader crash during expense processing");
    network.simulate_leader_crash().await;

    // Phase 3: Continue operations after recovery
    leader_info!("Phase 3: Post-recovery operations");
    for i in 0..2 {
        network
            .simulate_transaction(i + 1, i + 1, 15.0 + i as f64 * 3.0)
            .await;
    }

    fuel_info!("Expense consistency test completed");
}

#[tokio::test]
async fn test_high_volume_expenses() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    fuel_info!("TEST: High Volume Expense Processing");

    let network = TestNetwork::new(6, 4, 2).await;

    // Setup
    network.simulate_leader_election().await;
    network.wait(200).await;

    // Process many expenses quickly
    network.stress_test_transactions(15).await;

    // Verify system health after stress
    network.verify_system_health().await;

    fuel_info!("High volume expenses test completed");
}
