use crate::domain::client_operation::{
    policy::{self, OrderingScope, Recovery},
    registry::OperationIdentity,
};
use sha2::{Digest, Sha256};

pub(crate) fn identity(command: &str, mut args: serde_json::Value) -> OperationIdentity {
    if policy::recovery(command) == Recovery::CallerAttempt {
        if let Some(args) = args.as_object_mut() {
            args.remove("callerRequestId");
            if command == "request_application_quit" {
                if let Some(quit) = args
                    .get_mut("request")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    quit.remove("request_id");
                }
            }
        }
    }
    let fingerprint =
        Sha256::digest(serde_json::to_vec(&(command, &args)).expect("command identity")).into();
    let target = policy::ordering_scope(command).map(|scope| {
        use OrderingScope::*;
        let fields: &[&str] = match scope {
            RepositoryMembership => &["path"],
            WorkspaceState => &["worktreeName"],
            NotionConfiguration | RepositoryBase => &["repoPath"],
            BranchBase => &["repoPath", "branchName"],
            TerminalSize => &["owner"],
            TerminalOutput => &["attachmentId"],
            Watch => &["watcherId"],
            ApplicationSettings
            | ExternalEditor
            | WorkflowConfiguration
            | CrashReporting
            | PerformanceTelemetry
            | MountedTerminals => &[],
        };
        let values: Vec<_> = fields.iter().map(|key| &args[*key]).collect();
        (
            scope,
            Sha256::digest(
                serde_json::to_vec(&(format!("{scope:?}"), values)).expect("operation target"),
            )
            .into(),
        )
    });
    OperationIdentity {
        command: command.into(),
        fingerprint,
        target,
    }
}
