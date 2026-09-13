use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::client::invalid_request;
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::client::{outcome, ClientCommandDispatch};
pub(crate) fn register_shared(
    router: &mut ClientCommandDispatch,
    deps: &crate::adaptor::controller::client::ClientDependencies,
) {
    {
        let usecase = deps.watcher.clone();
        router.register_domain(
            &["start_watching"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StartWatching(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let input: String = convert(required(args.path, "path")?)?;
                        outcome(
                            tokio::task::spawn_blocking(move || usecase.start(&input))
                                .await
                                .map_err(|error| wire::CommandError::from(error.to_string()))?
                                .map_err(|error| error.to_string()),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::StartWatching(result))
                })
            }),
        );
    }
    {
        let usecase = deps.watcher.clone();
        router.register_domain(
            &["start_git_dir_watching"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StartGitDirWatching(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let input: String = convert(required(args.repo_path, "repoPath")?)?;
                        outcome(
                            tokio::task::spawn_blocking(move || usecase.start_git_dir(&input))
                                .await
                                .map_err(|error| wire::CommandError::from(error.to_string()))?
                                .map_err(|error| error.to_string()),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::StartGitDirWatching(result))
                })
            }),
        );
    }
    {
        let usecase = deps.watcher.clone();
        router.register_domain(
            &["stop_watching"],
            Box::new(move |command| {
                let usecase = usecase.clone();
                Box::pin(async move {
                    let wire::command_request::Command::StopWatching(args) = command else {
                        return Err(invalid_request("Mismatched command"));
                    };
                    let result = async move {
                        let input: u64 = convert(required(args.watcher_id, "watcherId")?)?;
                        outcome(
                            tokio::task::spawn_blocking(move || usecase.stop(input))
                                .await
                                .map_err(|error| wire::CommandError::from(error.to_string()))?
                                .map_err(|error| error.to_string()),
                        )
                    }
                    .await?;
                    Ok(wire::command_result::Command::StopWatching(result))
                })
            }),
        );
    }
}
