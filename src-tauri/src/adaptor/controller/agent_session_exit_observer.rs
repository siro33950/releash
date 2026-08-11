use std::sync::Arc;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventReceiveError, TerminalSurfaceEventStream,
};
use crate::usecase::agent_session::AgentSessionExitUsecase;

pub(crate) async fn run_agent_session_exit_observer(
    mut stream: TerminalSurfaceEventStream,
    usecase: Arc<AgentSessionExitUsecase>,
) {
    loop {
        match stream.subscription.recv().await {
            Ok(TerminalSurfaceEvent::Exit {
                session_key,
                runtime_generation,
                sequence,
                ..
            }) => {
                if let Err(error) = usecase
                    .observe_exit(
                        &session_key,
                        runtime_generation,
                        &format!("terminal-exit-{sequence}-{}", uuid::Uuid::new_v4()),
                    )
                    .await
                {
                    log::warn!("failed to observe AgentSession terminal exit: {error:?}");
                }
            }
            Ok(
                TerminalSurfaceEvent::Output { .. }
                | TerminalSurfaceEvent::Resize { .. }
                | TerminalSurfaceEvent::InputUnavailable { .. },
            ) => {}
            Err(TerminalSurfaceEventReceiveError::Lagged(_)) => {
                if let Err(error) = usecase
                    .reconcile_exited(&format!("terminal-exit-reconcile-{}", uuid::Uuid::new_v4()))
                    .await
                {
                    log::warn!("failed to reconcile AgentSession terminal exits: {error:?}");
                }
            }
            Err(TerminalSurfaceEventReceiveError::Closed) => break,
        }
    }
}

#[cfg(test)]
#[path = "agent_session_exit_observer_test.rs"]
mod agent_session_exit_observer_tests;
