use super::application::TerminalSurfaceApplication;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::usecase::state_subscription::{
    StateReadError, StateReadFailure, StateValue, SubscriptionError, SubscriptionTarget,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(crate) trait TerminalSubscriptionOutput: Send + Sync {
    fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
    ) -> Result<Option<usize>, StateReadError>;
    fn stop(&self, client: &str, target: &SubscriptionTarget) -> Result<(), SubscriptionError>;
    fn set_snapshot(
        &self,
        target: &SubscriptionTarget,
        generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError>;
    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError>;
}

#[derive(Clone)]
pub(crate) struct TerminalSubscriptionUsecase {
    publisher: Arc<dyn TerminalSubscriptionOutput>,
    terminal: Option<Arc<TerminalSurfaceApplication>>,
    clients: Arc<Mutex<HashMap<String, HashMap<SubscriptionTarget, String>>>>,
    terminal_resets: Arc<Mutex<HashMap<SubscriptionTarget, HashSet<String>>>>,
    workers: Arc<Mutex<HashMap<SubscriptionTarget, tokio::sync::oneshot::Sender<()>>>>,
    refresh_requests: tokio::sync::mpsc::UnboundedSender<TerminalRefresh>,
    refresh_events: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<TerminalRefresh>>>>,
}

pub(crate) struct TerminalRefresh {
    pub target: SubscriptionTarget,
    pub cancelled: tokio::sync::oneshot::Receiver<()>,
}

const TERMINAL_INPUT_ID_MAX_BYTES: usize = 128;

fn read_error(e: impl std::fmt::Display) -> StateReadError {
    StateReadError::from_error(TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message: e.to_string(),
    })
}

impl TerminalSubscriptionUsecase {
    pub(crate) fn new(
        publisher: Arc<dyn TerminalSubscriptionOutput>,
        terminal: Option<Arc<TerminalSurfaceApplication>>,
    ) -> Self {
        let (refresh_requests, refresh_events) = tokio::sync::mpsc::unbounded_channel();
        Self {
            refresh_requests,
            refresh_events: Arc::new(Mutex::new(Some(refresh_events))),
            publisher,
            terminal,
            clients: Default::default(),
            terminal_resets: Default::default(),
            workers: Default::default(),
        }
    }

    pub(crate) fn open_client(&self, client: String) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        if clients.contains_key(&client) {
            return Err(SubscriptionError::AlreadyExists);
        }
        clients.insert(client, HashMap::new());
        Ok(())
    }

    pub(crate) fn close_client(&self, client: &str) {
        let targets = self.clients.lock().remove(client).unwrap_or_default();
        for (target, input) in targets {
            self.unsubscribe(client, &target, &input);
        }
        self.terminal_resets
            .lock()
            .values_mut()
            .for_each(|clients| {
                clients.remove(client);
            });
        self.stop_inactive_workers();
    }

    fn stop_inactive_workers(&self) {
        let clients = self.clients.lock();
        let is_subscribed = |target: &SubscriptionTarget| {
            clients.values().any(|targets| targets.contains_key(target))
        };
        self.terminal_resets
            .lock()
            .retain(|target, _| is_subscribed(target));
        self.workers
            .lock()
            .retain(|target, _cancel| is_subscribed(target));
    }

    fn unsubscribe(&self, client: &str, target: &SubscriptionTarget, input: &str) {
        if let (Some(terminal), SubscriptionTarget::Terminal(owner)) = (&self.terminal, target) {
            terminal.unsubscribe_output(owner, client, input);
        }
    }

    pub(crate) fn stop_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        let input = self
            .clients
            .lock()
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?
            .remove(target);
        if let Some(input) = input {
            self.unsubscribe(client, target, &input);
        }
        self.stop_inactive_workers();
        self.publisher.stop(client, target)
    }

    pub async fn start_terminal(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: Option<&str>,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        let input_id = input_id.unwrap_or(client);
        if input_id.trim().is_empty() || input_id.len() > TERMINAL_INPUT_ID_MAX_BYTES {
            return Err(StateReadError {
                source: StateReadFailure::InvalidTerminalInput,
                message: "Invalid terminal input identity".into(),
            });
        }
        let SubscriptionTarget::Terminal(_) = target else {
            return Err(read_error("Not a terminal target"));
        };
        self.terminal
            .as_ref()
            .ok_or_else(|| read_error("Terminal unavailable"))?;
        self.clients
            .lock()
            .get_mut(client)
            .ok_or_else(|| StateReadError::from_error(SubscriptionError::StreamEnded))?
            .insert(target.clone(), input_id.into());
        if let Err(error) = self.present_start(client, target, input_id, cursor) {
            let _ = self.stop_subscription(client, target);
            return Err(error);
        }
        if let Err(error) = self.ensure_subscribed(client, target) {
            self.unsubscribe(client, target, input_id);
            let _ = self.publisher.stop(client, target);
            return Err(error);
        }
        Ok(())
    }

    fn present_start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = target else {
            return Err(read_error("Not a terminal target"));
        };
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| read_error("Terminal unavailable"))?;
        loop {
            let generation = terminal
                .get_summary(owner)
                .map_err(StateReadError::from_error)?
                .runtime_generation;
            let mut result = None;
            let entered = terminal.with_output_order(generation.value(), &mut || {
                match terminal.get_summary(owner) {
                    Ok(current) if current.runtime_generation != generation => return,
                    Err(error) => {
                        result = Some(Err(StateReadError::from_error(error)));
                        return;
                    }
                    _ => {}
                }
                result = Some(self.publisher.start(client, target, cursor).map(|pending| {
                    terminal.subscribe_output(owner, client, input_id, pending);
                }));
            });
            if !entered {
                continue;
            }
            if let Some(result) = result {
                return result;
            }
        }
    }

    fn ensure_subscribed(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        self.clients
            .lock()
            .get(client)
            .filter(|targets| targets.contains_key(target))
            .ok_or_else(|| StateReadError::from_error(SubscriptionError::StreamEnded))?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_input_id(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Option<String> {
        self.clients
            .lock()
            .get(client)
            .and_then(|targets| targets.get(target))
            .cloned()
    }

    pub(crate) async fn refresh_terminal(
        &self,
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        let SubscriptionTarget::Terminal(owner) = target else {
            return Ok(());
        };
        let owner = owner.clone();
        let terminal = self
            .terminal
            .clone()
            .ok_or_else(|| read_error("Terminal unavailable"))?;
        let output = self.publisher.clone();
        let target = target.clone();
        let resets = self.terminal_resets.clone();
        let inputs = self.clients.clone();
        tokio::task::spawn_blocking(move || {
            let mut result = Ok(());
            terminal
                .visit_snapshot(&owner, &mut |surface| {
                    let reset = resets.lock().remove(&target).unwrap_or_default();
                    {
                        let active = inputs.lock();
                        for client in reset {
                            if active
                                .get(&client)
                                .is_some_and(|targets| targets.contains_key(&target))
                            {
                                terminal.reset_output(&owner, &client);
                            }
                        }
                    }
                    result = output.set_snapshot(
                        &target,
                        surface.runtime_generation.value(),
                        surface.latest_sequence(),
                        StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface.into())),
                    );
                })
                .map_err(StateReadError::from_error)?;
            result.map_err(read_error)
        })
        .await
        .map_err(read_error)?
    }

    pub(crate) fn schedule_terminal_refresh(
        &self,
        clients: Vec<String>,
        target: SubscriptionTarget,
    ) {
        if !matches!(target, SubscriptionTarget::Terminal(_)) {
            return;
        }
        self.terminal_resets
            .lock()
            .entry(target.clone())
            .or_default()
            .extend(clients);
        let mut workers = self.workers.lock();
        if workers.contains_key(&target) {
            return;
        }
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        workers.insert(target.clone(), cancel);
        if self
            .refresh_requests
            .send(TerminalRefresh {
                target: target.clone(),
                cancelled,
            })
            .is_err()
        {
            workers.remove(&target);
            drop(workers);
            if let Err(error) = self.publisher.publish_failure(
                &target,
                StateReadError::from_error(SubscriptionError::StreamEnded),
            ) {
                log::error!("Terminal failure publication failed: {error}");
            }
        }
    }

    pub(crate) fn take_refresh_events(
        &self,
    ) -> tokio::sync::mpsc::UnboundedReceiver<TerminalRefresh> {
        self.refresh_events
            .lock()
            .take()
            .expect("terminal refresh driver")
    }

    pub(crate) async fn refresh_terminal_once(&self, target: &SubscriptionTarget) -> bool {
        let result = self.refresh_terminal(target).await;
        let failed = result.is_err();
        if let Err(error) = result {
            if let Err(error) = self.publisher.publish_failure(target, error) {
                log::error!("Terminal failure publication failed: {error}");
            }
        }
        let resets = self.terminal_resets.lock();
        if failed || resets.get(target).is_none_or(HashSet::is_empty) {
            self.workers.lock().remove(target);
            false
        } else {
            true
        }
    }

    #[cfg(test)]
    pub(crate) fn test_worker_count(&self) -> usize {
        self.workers.lock().len()
    }

    pub fn terminal_processed(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        units: usize,
    ) -> Result<(), StateReadError> {
        if !self
            .clients
            .lock()
            .get(client)
            .is_some_and(|targets| targets.contains_key(target))
        {
            return Err(StateReadError {
                source: StateReadFailure::TerminalSubscriptionEnded,
                message: "Terminal subscription ended".into(),
            });
        }
        let SubscriptionTarget::Terminal(owner) = target else {
            return Err(read_error("Not a terminal target"));
        };
        self.terminal
            .as_ref()
            .ok_or_else(|| read_error("Terminal unavailable"))?
            .processed_output(owner, client, units);
        Ok(())
    }
}

#[cfg(test)]
#[path = "subscription_test.rs"]
mod subscription_tests;
