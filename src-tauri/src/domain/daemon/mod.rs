mod failure;
pub(crate) mod identity;
pub(crate) use failure::{StartupFailure, StartupFailureKind};
pub(crate) use identity::{
    ConnectionObservation, DaemonIdentity, DiscoveryRejection, ProcessObservation,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServingStatus {
    Starting,
    Serving,
    Stopping,
    Stopped,
    Failed(StartupFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonRequest {
    Status,
    Stop,
    Operation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRequest {
    Exit { code: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAcceptance {
    Started { code: i32 },
    AlreadyAccepted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonInfo {
    pub(crate) identity: DaemonIdentity,
    pub(crate) release: String,
    pub(crate) protocol: u32,
    pub(crate) capabilities: BTreeSet<String>,
    pub(crate) serving_status: ServingStatus,
}

pub(crate) struct Daemon {
    info: DaemonInfo,
    exit_code: Option<i32>,
}

impl Daemon {
    pub(crate) fn new(identity: DaemonIdentity, release: String, protocol: u32) -> Self {
        Self {
            info: DaemonInfo {
                identity,
                release,
                protocol,
                capabilities: BTreeSet::new(),
                serving_status: ServingStatus::Starting,
            },
            exit_code: None,
        }
    }
    pub(crate) fn info(&self) -> DaemonInfo {
        self.info.clone()
    }
    pub(crate) fn admits(&self, request: DaemonRequest) -> bool {
        matches!(request, DaemonRequest::Status | DaemonRequest::Stop)
            || self.info.serving_status == ServingStatus::Serving
    }
    pub(crate) fn serve(&mut self) {
        if self.info.serving_status == ServingStatus::Starting {
            self.info.serving_status = ServingStatus::Serving;
        }
    }
    pub(crate) fn fail(&mut self, failure: StartupFailure) {
        if self.info.serving_status == ServingStatus::Starting {
            self.info.serving_status = ServingStatus::Failed(failure);
        }
    }
    pub(crate) fn accepted_stop(&self) -> Option<StopAcceptance> {
        self.exit_code.map(|_| StopAcceptance::AlreadyAccepted)
    }
    pub(crate) fn stop(&mut self, StopRequest::Exit { code }: StopRequest) -> StopAcceptance {
        if let Some(acceptance) = self.accepted_stop() {
            return acceptance;
        }
        self.exit_code = Some(code);
        self.info.serving_status = ServingStatus::Stopping;
        StopAcceptance::Started { code }
    }
    pub(crate) fn stopped(&mut self) {
        if self.info.serving_status == ServingStatus::Stopping {
            self.info.serving_status = ServingStatus::Stopped;
        }
    }
}

#[async_trait::async_trait]
pub trait DaemonRepository: Send + Sync {
    async fn info(&self) -> DaemonInfo;
    async fn admits(&self, request: DaemonRequest) -> bool;
    async fn serve(&self);
    async fn fail(&self, failure: StartupFailure);
    async fn stop(&self, request: StopRequest) -> StopAcceptance;
    async fn stopped(&self);
}

#[cfg(test)]
#[path = "daemon_test.rs"]
pub(crate) mod daemon_tests;
