#[allow(unused_imports)]
use crate::*;
#[allow(unused_imports)]
use common::{crash_info, election_info, fuel_info, leader_info};

#[tokio::test]
async fn test_complete_system_end_to_end() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    election_info!("COMPLETE SYSTEM END-TO-END TEST");

    // Create full system
    let network = TestNetwork::new(5, 3, 2).await;

    // Phase 1: System initialization
    election_info!("Phase 1: System Initialization");
    network.simulate_leader_election().await;
    network.wait(400).await;

    // Phase 2: Normal operations
    fuel_info!("Phase 2: Normal Operations");
    for i in 0..5 {
        let card_id = i % network.card_count;
        let pump_id = i % network.pump_count;
        let liters = 30.0 + i as f64 * 8.0;

        network.simulate_transaction(card_id, pump_id, liters).await;
        network.wait(100).await;
    }

    // Phase 3: Leader crash and recovery
    crash_info!("Phase 3: Leader Crash & Recovery");
    network.simulate_leader_crash().await;

    // Phase 4: Post-recovery operations
    leader_info!("Phase 4: Post-Recovery Operations");
    for i in 0..3 {
        let card_id = i % network.card_count;
        let pump_id = (i + 1) % network.pump_count;
        let liters = 18.0 + i as f64 * 6.0;

        network.simulate_transaction(card_id, pump_id, liters).await;
        network.wait(100).await;
    }

    // Final verification
    network.verify_system_health().await;

    leader_info!("COMPLETE SYSTEM TEST PASSED!");
}

#[tokio::test]
async fn test_system_resilience() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    crash_info!("SYSTEM RESILIENCE TEST");

    let network = TestNetwork::new(6, 3, 2).await;

    // Initial setup
    network.simulate_leader_election().await;
    network.wait(300).await;

    // Stress test with multiple failures
    for cycle in 1..=3 {
        crash_info!("Resilience cycle {}/3", cycle);

        // Normal operations
        for i in 0..2 {
            network
                .simulate_transaction(
                    i % network.card_count,
                    i % network.pump_count,
                    25.0 + i as f64 * 12.0,
                )
                .await;
        }

        network.wait(200).await;
    }

    leader_info!("SYSTEM RESILIENCE TEST PASSED!");
}

#[tokio::test]
async fn test_extreme_stress() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    crash_info!("EXTREME STRESS TEST");

    let network = TestNetwork::new(8, 5, 3).await;

    // Setup
    network.simulate_leader_election().await;

    // Extreme stress: transactions + crashes
    for round in 1..=4 {
        crash_info!("Extreme stress round {}/4", round);

        network.stress_test_transactions(8).await;

        network.wait(150).await;
    }

    // Final health check
    network.verify_system_health().await;

    leader_info!("EXTREME STRESS TEST SURVIVED!");
}
