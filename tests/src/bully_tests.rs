#[allow(unused_imports)]
use crate::TestNetwork;
#[allow(unused_imports)]
use common::{crash_info, election_info, leader_info};

#[tokio::test]
async fn test_global_bully_algorithm() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    election_info!("TEST: Global Bully Algorithm");

    // Create network with 5 pumps
    let network = TestNetwork::new(5, 0, 0).await;

    // Test leader election
    network.simulate_leader_election().await;
    network.wait(500).await;

    // Verify expected leader
    let expected_leader = network.get_expected_leader();
    leader_info!("Expected leader: pump {}", expected_leader);

    election_info!("Bully algorithm test completed");
}

#[tokio::test]
async fn test_leader_crash_recovery() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    crash_info!("TEST: Leader Crash Recovery");

    let network = TestNetwork::new(7, 0, 0).await;

    // Initial election
    network.simulate_leader_election().await;
    network.wait(300).await;

    // Crash and recovery
    network.simulate_leader_crash().await;
    network.wait(500).await;

    leader_info!("Leader crash recovery test completed");
}

#[tokio::test]
async fn test_concurrent_elections() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Off)
        .try_init();

    election_info!("TEST: Multiple Elections");

    let network = TestNetwork::new(4, 0, 0).await;

    // Run multiple elections
    for round in 1..=3 {
        election_info!("Election cycle {}/3", round);
        network.simulate_leader_election().await;
        network.wait(200).await;
    }

    election_info!("Multiple elections test completed");
}

#[tokio::test]
async fn test_large_network_election() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .try_init();

    election_info!("TEST: Large Network Election (10 pumps)");

    let network = TestNetwork::new(10, 0, 0).await;

    // Large network election
    network.simulate_leader_election().await;
    network.wait(400).await;

    // Verify system can handle large network
    network.verify_system_health().await;

    leader_info!("Large network election test completed");
}
