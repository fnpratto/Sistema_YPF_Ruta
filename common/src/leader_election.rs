// Remove the log import since we'll use custom macros
use crate::{election_debug, election_info, leader_info};
use std::collections::HashMap;
use std::net::UdpSocket;
use tokio::sync::mpsc::{self as async_mpsc, Sender, UnboundedReceiver, UnboundedSender}; // Add Sender here
use tokio::task;
use tokio::time::{Duration, sleep};

const TIMEOUT: Duration = Duration::from_secs(1);

fn id_to_ctrladdr(id: usize) -> String {
    format!("127.0.0.1:{}", id as u16)
}

#[derive(Debug, Clone)]
pub enum LeaderEvent {
    Election(usize),    // Election started by given id
    Coordinator(usize), // Coordinator elected with given id
}

#[derive(Debug)]
pub enum LeaderMessage {
    StartElection,
    Stop,
    Reset,
    UpdateMembers {
        local_addr: String,
        pumps: HashMap<String, bool>,
    },
    ReceivedElection {
        id_from: usize,
    },
    ReceivedCoordinator {
        id_from: usize,
    },
    ReceivedOk {
        id_from: usize,
    },
}
pub struct LeaderElection {
    id: usize,
    socket: UdpSocket,
    leader_id: Option<usize>,
    peers: Vec<u32>,
    pumps: HashMap<String, bool>,
    /// An optional channel sender used to notify about leader election events.
    /// If present, it allows sending `LeaderEvent` messages to inform about changes
    /// in the leader's state or other related events.
    leader_notify: Option<Sender<LeaderEvent>>, // Changed from std::sync::mpsc::Sender to tokio::sync::mpsc::Sender
    /// An unbounded sender used for sending `LeaderMessage` instances to a mailbox.
    /// This is part of the leader election mechanism, allowing communication
    /// between components involved in the election process.
    mailbox: UnboundedSender<LeaderMessage>,
}

impl Clone for LeaderElection {
    fn clone(&self) -> Self {
        LeaderElection {
            id: self.id,
            socket: self.socket.try_clone().expect("Failed to clone UdpSocket"),
            leader_id: self.leader_id,
            peers: self.peers.clone(),
            pumps: self.pumps.clone(),
            leader_notify: self.leader_notify.clone(),
            mailbox: self.mailbox.clone(),
        }
    }
}

impl LeaderElection {
    pub fn new(
        local_addr: &str,
        pumps: HashMap<String, bool>,
        leader_notify: Option<Sender<LeaderEvent>>, // Changed parameter type
    ) -> Result<LeaderElection, String> {
        let mut ports: Vec<u32> = pumps.keys().filter_map(|k| k.parse::<u32>().ok()).collect();
        ports.sort_unstable();

        let local_port = local_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(0);

        let id = local_port as usize;

        let socket = UdpSocket::bind(id_to_ctrladdr(id))
            .map_err(|e| format!("Failed to bind UdpSocket: {}", e))?;

        let (mailbox_tx, mailbox_rx) = async_mpsc::unbounded_channel();

        let actor = LeaderElection {
            id,
            socket,
            leader_id: Some(id),
            peers: ports,
            pumps,
            leader_notify,
            mailbox: mailbox_tx.clone(),
        };

        // Clone the actor for the task
        let actor_clone = LeaderElection {
            id: actor.id,
            socket: actor.socket.try_clone().unwrap(),
            leader_id: actor.leader_id,
            peers: actor.peers.clone(),
            pumps: actor.pumps.clone(),
            leader_notify: actor.leader_notify.clone(),
            mailbox: actor.mailbox.clone(),
        };

        // Spawn the actor task
        task::spawn(Self::run_actor(actor_clone, mailbox_rx));

        Ok(actor)
    }

    #[allow(unused_assignments)]
    async fn run_actor(mut self, mut mailbox: UnboundedReceiver<LeaderMessage>) {
        let mut got_ok = false;
        let mut election_in_progress = false;
        let mut leader_elected = false;

        while let Some(message) = mailbox.recv().await {
            match message {
                LeaderMessage::StartElection => {
                    if leader_elected {
                        election_info!("[{}] Election ignored - leader already elected", self.id);
                        continue;
                    }
                    if election_in_progress {
                        election_info!("[{}] Election already in progress", self.id);
                        continue;
                    }

                    election_info!("[{}] Starting election", self.id);
                    election_in_progress = true;
                    got_ok = false;
                    self.leader_id = None;

                    // Send election message to higher ID nodes only
                    let has_higher_peers = self.send_election().await;

                    if !has_higher_peers {
                        // No higher ID peers, become leader immediately
                        self.make_me_leader().await;
                        leader_elected = true;
                        election_in_progress = false;
                    } else {
                        // Wait for responses from higher ID peers
                        sleep(TIMEOUT).await;
                        if !got_ok {
                            self.make_me_leader().await;
                            leader_elected = true;
                        }
                        election_in_progress = false;
                    }
                }
                LeaderMessage::Reset => {
                    election_info!("[{}] Resetting election state", self.id);
                    leader_elected = false;
                    election_in_progress = false;
                    got_ok = false;
                    self.leader_id = None;
                }
                LeaderMessage::ReceivedOk { id_from } => {
                    election_info!("[{}] Received OK from {}", self.id, id_from);
                    got_ok = true;
                }
                LeaderMessage::ReceivedCoordinator { id_from } => {
                    if let Some(current_leader) = self.leader_id
                        && current_leader == id_from
                    {
                        election_info!(
                            "[{}] Duplicate coordinator for {} ignored",
                            self.id,
                            id_from
                        );
                        continue;
                    }

                    leader_info!("[{}] Received Coordinator from {}", self.id, id_from);
                    self.leader_id = Some(id_from);
                    leader_elected = true;
                    election_in_progress = false;

                    if let Some(sender) = &self.leader_notify {
                        let _ = sender.send(LeaderEvent::Coordinator(id_from)).await;
                    }
                }
                LeaderMessage::ReceivedElection { id_from } => {
                    if leader_elected {
                        election_info!("[{}] Ignoring election - leader already elected", self.id);
                        continue;
                    }
                    election_info!("[{}] Received Election from {}", self.id, id_from);
                    if id_from < self.id {
                        self.send_ok(id_from).await;
                        if !election_in_progress {
                            let _ = self.mailbox.send(LeaderMessage::StartElection);
                        }
                    }
                }
                LeaderMessage::Stop => {
                    election_info!("[{}] Election stopped", self.id);
                    break; // Exit the actor loop
                }
                LeaderMessage::UpdateMembers { local_addr, pumps } => {
                    self.update_members(local_addr, pumps).await;
                }
            }
        }

        election_info!("[{}] Election actor terminated", self.id);
    }

    async fn send_election(&self) -> bool {
        let msg = self.id_to_msg(b'E');
        let mut has_higher_peers = false;

        if let Some(sender) = &self.leader_notify {
            let _ = sender.send(LeaderEvent::Election(self.id)).await;
        }

        for &peer_port in &self.peers {
            let peer_id = peer_port as usize;
            if peer_id > self.id {
                has_higher_peers = true;
                self.socket
                    .send_to(&msg, id_to_ctrladdr(peer_id))
                    .unwrap_or_default();
            }
        }

        has_higher_peers
    }

    async fn make_me_leader(&mut self) {
        leader_info!("[{}] Announcing myself as leader", self.id);
        let msg = self.id_to_msg(b'C');
        for &peer_port in &self.peers {
            let peer_id = peer_port as usize;
            if peer_id != self.id {
                self.socket
                    .send_to(&msg, id_to_ctrladdr(peer_id))
                    .unwrap_or_default();
            }
        }
        self.leader_id = Some(self.id);
        if let Some(sender) = &self.leader_notify {
            let _ = sender.send(LeaderEvent::Coordinator(self.id)).await; // Changed to async send
        }
    }

    async fn send_ok(&self, id_from: usize) {
        let msg = self.id_to_msg(b'O');
        self.socket
            .send_to(&msg, id_to_ctrladdr(id_from))
            .unwrap_or_default();
    }

    async fn update_members(&mut self, local_addr: String, pumps: HashMap<String, bool>) {
        let mut ports: Vec<u32> = pumps.keys().filter_map(|k| k.parse::<u32>().ok()).collect();
        ports.sort_unstable();
        let local_port_opt = local_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u32>().ok());
        let new_id = if let Some(local_port) = local_port_opt {
            local_port as usize
        } else {
            0
        };

        if new_id != self.id {
            self.socket = UdpSocket::bind(id_to_ctrladdr(new_id)).unwrap();
            self.id = new_id;
        }
        self.peers = ports;
        self.pumps = pumps;
    }

    fn id_to_msg(&self, header: u8) -> Vec<u8> {
        let mut msg = vec![header];
        msg.extend_from_slice(&self.id.to_le_bytes());
        msg
    }

    pub fn trigger_election(&self) {
        election_info!("[{}] TRIGGER_ELECTION called", self.id);

        election_info!("[{}] Sending StartElection message to actor", self.id);

        if let Err(e) = self.mailbox.send(LeaderMessage::StartElection) {
            election_info!("[{}] Failed to send StartElection: {}", self.id, e);
        } else {
            election_info!("[{}] StartElection message sent to actor", self.id);
        }
    }

    pub fn stop(&self) {
        election_debug!("[{}] STOP called", self.id);

        let _ = self.mailbox.send(LeaderMessage::Stop);
    }

    pub fn update_members_sync(&self, local_addr: String, pumps: HashMap<String, bool>) {
        let _ = self
            .mailbox
            .send(LeaderMessage::UpdateMembers { local_addr, pumps });
    }

    pub fn receive_coordinator(&self, leader_id: usize) {
        election_info!("[{}] RECEIVE_COORDINATOR: {}", self.id, leader_id);

        let _ = self
            .mailbox
            .send(LeaderMessage::ReceivedCoordinator { id_from: leader_id });
    }

    pub fn reset(&self) {
        election_info!("[{}] RESET called - clearing leader state", self.id);

        let _ = self.mailbox.send(LeaderMessage::Reset);
    }
}
