use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use yrr_core::error::{Result, YrrError};
use yrr_core::message::SignalMessage;

use crate::bus::{BusQuery, SignalBus};
use crate::mapper::SignalMapper;

/// Zenoh-backed implementation of SignalBus.
pub struct ZenohBus {
    session: zenoh::Session,
    mapper: SignalMapper,
}

impl ZenohBus {
    /// Open a new ZenohBus with the given namespace (typically the swarm name).
    pub async fn new(namespace: impl Into<String>) -> Result<Self> {
        let session = zenoh::open(zenoh::Config::default())
            .await
            .map_err(|e| YrrError::Bus(format!("failed to open zenoh session: {e}")))?;

        Ok(Self {
            session,
            mapper: SignalMapper::new(namespace),
        })
    }

    /// Open with an existing Zenoh config.
    pub async fn with_config(namespace: impl Into<String>, config: zenoh::Config) -> Result<Self> {
        let session = zenoh::open(config)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to open zenoh session: {e}")))?;

        Ok(Self {
            session,
            mapper: SignalMapper::new(namespace),
        })
    }
}

#[async_trait]
impl SignalBus for ZenohBus {
    async fn publish(&self, signal: &str, message: &SignalMessage) -> Result<()> {
        let key = self.mapper.signal_to_key(signal);
        let payload = serde_json::to_vec(message).map_err(|e| YrrError::Bus(e.to_string()))?;

        debug!(
            signal,
            key,
            source = %message.source_agent_name,
            correlation_id = %message.correlation_id,
            "publishing signal"
        );

        self.session
            .put(&key, payload)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to publish to {key}: {e}")))?;

        Ok(())
    }

    async fn subscribe(&self, signal: &str) -> Result<mpsc::Receiver<SignalMessage>> {
        let key = self.mapper.signal_to_key(signal);
        let (tx, rx) = mpsc::channel(64);

        info!(signal, key, "subscribing to signal");

        let subscriber = self
            .session
            .declare_subscriber(&key)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to subscribe to {key}: {e}")))?;

        // Spawn a task that forwards Zenoh samples to the mpsc channel.
        tokio::spawn(async move {
            loop {
                match subscriber.recv_async().await {
                    Ok(sample) => {
                        let bytes = sample.payload().to_bytes();
                        match serde_json::from_slice::<SignalMessage>(&bytes) {
                            Ok(msg) => {
                                if tx.send(msg).await.is_err() {
                                    // Receiver dropped — stop forwarding.
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("failed to deserialize signal message: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        debug!("subscriber closed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn declare_queryable(&self, key: &str) -> Result<mpsc::Receiver<BusQuery>> {
        let zenoh_key = self.mapper.queryable_to_key(key);
        let (tx, rx) = mpsc::channel(64);
        let queryable_key = key.to_string();

        info!(key, zenoh_key = %zenoh_key, "declaring queryable");

        let queryable = self
            .session
            .declare_queryable(&zenoh_key)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to declare queryable {zenoh_key}: {e}")))?;

        tokio::spawn(async move {
            loop {
                match queryable.recv_async().await {
                    Ok(query) => {
                        let payload = query
                            .payload()
                            .map(|p| String::from_utf8(p.to_bytes().to_vec()).unwrap_or_default())
                            .unwrap_or_default();

                        let key_expr = query.key_expr().to_string();
                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

                        let bus_query = BusQuery::new(queryable_key.clone(), payload, reply_tx);

                        if tx.send(bus_query).await.is_err() {
                            break; // receiver dropped
                        }

                        // Wait for the reply from the handler and send it back via Zenoh.
                        match reply_rx.await {
                            Ok(Ok(response)) => {
                                if let Err(e) = query.reply(&key_expr, response).await {
                                    error!("failed to send query reply: {e}");
                                }
                            }
                            Ok(Err(err_msg)) => {
                                if let Err(e) = query.reply_err(err_msg).await {
                                    error!("failed to send query error reply: {e}");
                                }
                            }
                            Err(_) => {
                                // reply_tx was dropped without sending — agent failed
                                let _ = query.reply_err("handler dropped without replying").await;
                            }
                        }
                    }
                    Err(e) => {
                        debug!("queryable closed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn query(&self, key: &str, payload: &str, timeout: Duration) -> Result<String> {
        let zenoh_key = self.mapper.queryable_to_key(key);

        debug!(key, zenoh_key = %zenoh_key, "issuing query");

        let replies = self
            .session
            .get(&zenoh_key)
            .payload(payload)
            .timeout(timeout)
            .await
            .map_err(|e| YrrError::Query(format!("query to {key} failed: {e}")))?;

        // Collect the first reply.
        match replies.recv_async().await {
            Ok(reply) => match reply.result() {
                Ok(sample) => {
                    let bytes = sample.payload().to_bytes();
                    String::from_utf8(bytes.to_vec())
                        .map_err(|e| YrrError::Query(format!("invalid UTF-8 in reply: {e}")))
                }
                Err(err) => {
                    let bytes = err.payload().to_bytes();
                    let msg = String::from_utf8_lossy(&bytes);
                    Err(YrrError::Query(format!("queryable returned error: {msg}")))
                }
            },
            Err(_) => Err(YrrError::Query(format!(
                "no reply received for query to {key} (timeout or no queryable)"
            ))),
        }
    }

    async fn publish_status(&self, agent_id: &str, status: &str) -> Result<()> {
        let key = self.mapper.status_key(agent_id);

        debug!(agent_id, status, key = %key, "publishing agent status");

        self.session
            .put(&key, status)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to publish status to {key}: {e}")))?;

        Ok(())
    }

    async fn subscribe_status(&self) -> Result<mpsc::Receiver<(String, String)>> {
        let key = self.mapper.status_wildcard();
        let (tx, rx) = mpsc::channel(128);

        info!(key = %key, "subscribing to agent status updates");

        let subscriber = self
            .session
            .declare_subscriber(&key)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to subscribe to status {key}: {e}")))?;

        let mapper = SignalMapper::new(self.mapper.namespace());

        tokio::spawn(async move {
            loop {
                match subscriber.recv_async().await {
                    Ok(sample) => {
                        let key_str = sample.key_expr().to_string();
                        let status = String::from_utf8(sample.payload().to_bytes().to_vec())
                            .unwrap_or_default();

                        if let Some(agent_id) = mapper.key_to_agent_id(&key_str) {
                            if tx.send((agent_id.to_string(), status)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        debug!("status subscriber closed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn dispatch_to(&self, agent_id: &str, message: &SignalMessage) -> Result<()> {
        let key = self.mapper.dispatch_key(agent_id);
        let payload = serde_json::to_vec(message).map_err(|e| YrrError::Bus(e.to_string()))?;

        debug!(
            agent_id,
            signal = %message.signal,
            key = %key,
            "dispatching signal to agent"
        );

        self.session
            .put(&key, payload)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to dispatch to {key}: {e}")))?;

        Ok(())
    }

    async fn subscribe_dispatch(&self, agent_id: &str) -> Result<mpsc::Receiver<SignalMessage>> {
        let key = self.mapper.dispatch_key(agent_id);
        let (tx, rx) = mpsc::channel(64);

        info!(agent_id, key = %key, "subscribing to dispatch channel");

        let subscriber =
            self.session.declare_subscriber(&key).await.map_err(|e| {
                YrrError::Bus(format!("failed to subscribe to dispatch {key}: {e}"))
            })?;

        tokio::spawn(async move {
            loop {
                match subscriber.recv_async().await {
                    Ok(sample) => {
                        let bytes = sample.payload().to_bytes();
                        match serde_json::from_slice::<SignalMessage>(&bytes) {
                            Ok(msg) => {
                                if tx.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("failed to deserialize dispatched message: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        debug!("dispatch subscriber closed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn publish_steer(&self, agent_name: &str, payload: &str) -> Result<()> {
        let key = self.mapper.steer_key(agent_name);

        debug!(agent_name, key = %key, "publishing steer message");

        self.session
            .put(&key, payload)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to publish steer to {key}: {e}")))?;

        Ok(())
    }

    async fn subscribe_steer(&self, agent_name: &str) -> Result<mpsc::Receiver<String>> {
        let key = self.mapper.steer_key(agent_name);
        let (tx, rx) = mpsc::channel(16);

        info!(agent_name, key = %key, "subscribing to steer channel");

        let subscriber = self
            .session
            .declare_subscriber(&key)
            .await
            .map_err(|e| YrrError::Bus(format!("failed to subscribe to steer {key}: {e}")))?;

        tokio::spawn(async move {
            loop {
                match subscriber.recv_async().await {
                    Ok(sample) => {
                        let payload = String::from_utf8(sample.payload().to_bytes().to_vec())
                            .unwrap_or_default();

                        if tx.send(payload).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        debug!("steer subscriber closed: {e}");
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn close(&self) -> Result<()> {
        info!("closing zenoh bus");
        self.session
            .close()
            .await
            .map_err(|e| YrrError::Bus(format!("failed to close zenoh session: {e}")))?;
        Ok(())
    }
}
