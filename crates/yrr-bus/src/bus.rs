use std::time::Duration;

use async_trait::async_trait;
use yrr_core::error::Result;
use yrr_core::message::SignalMessage;
use tokio::sync::{mpsc, oneshot};

/// An incoming query from the bus. The holder must call `reply()` to respond.
pub struct BusQuery {
    /// The queryable key that was targeted.
    pub key: String,
    /// The payload sent by the querier.
    pub payload: String,
    /// Oneshot channel to send the reply back through the bus.
    reply_tx: oneshot::Sender<std::result::Result<String, String>>,
}

impl BusQuery {
    pub fn new(
        key: String,
        payload: String,
        reply_tx: oneshot::Sender<std::result::Result<String, String>>,
    ) -> Self {
        Self {
            key,
            payload,
            reply_tx,
        }
    }

    /// Send a successful reply. Consumes self.
    pub fn reply(self, response: String) -> std::result::Result<(), String> {
        self.reply_tx
            .send(Ok(response))
            .map_err(|_| "reply channel closed".to_string())
    }

    /// Send an error reply. Consumes self.
    pub fn reply_err(self, error: String) -> std::result::Result<(), String> {
        self.reply_tx
            .send(Err(error))
            .map_err(|_| "reply channel closed".to_string())
    }
}

/// Abstraction over the messaging system.
/// Implementations: ZenohBus (and potentially others in the future).
#[async_trait]
pub trait SignalBus: Send + Sync {
    /// Publish a signal message to the bus.
    async fn publish(&self, signal: &str, message: &SignalMessage) -> Result<()>;

    /// Subscribe to a signal. Returns a receiver that yields messages
    /// published to that signal.
    async fn subscribe(&self, signal: &str) -> Result<mpsc::Receiver<SignalMessage>>;

    /// Declare a queryable endpoint. Returns a receiver that yields incoming queries.
    async fn declare_queryable(&self, key: &str) -> Result<mpsc::Receiver<BusQuery>>;

    /// Issue a query and wait for a reply.
    async fn query(&self, key: &str, payload: &str, timeout: Duration) -> Result<String>;

    /// Publish agent status (idle/busy) to a dedicated key expression.
    async fn publish_status(&self, agent_id: &str, status: &str) -> Result<()>;

    /// Subscribe to all agent status updates in this namespace.
    /// Returns (agent_id, status) pairs.
    async fn subscribe_status(&self) -> Result<mpsc::Receiver<(String, String)>>;

    /// Publish a signal message to a specific agent's dispatch channel.
    async fn dispatch_to(&self, agent_id: &str, message: &SignalMessage) -> Result<()>;

    /// Subscribe to dispatched messages for a specific agent.
    async fn subscribe_dispatch(&self, agent_id: &str) -> Result<mpsc::Receiver<SignalMessage>>;

    /// Publish a steer message to a named agent.
    async fn publish_steer(&self, agent_name: &str, payload: &str) -> Result<()>;

    /// Subscribe to steer messages for a named agent.
    async fn subscribe_steer(&self, agent_name: &str) -> Result<mpsc::Receiver<String>>;

    /// Close the bus and clean up resources.
    async fn close(&self) -> Result<()>;
}
