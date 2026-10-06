pub(crate) mod tests_support {
    use super::super::*;
    use tokio::sync::mpsc;

    pub struct TestRepositoryStateWorkerRuntime;

    struct TokioInvalidationSender(mpsc::UnboundedSender<InvalidateReason>);

    impl RepositoryStateInvalidationSender for TokioInvalidationSender {
        fn send(&self, reason: InvalidateReason) -> Result<(), RepositoryStateError> {
            self.0.send(reason).map_err(|_| {
                RepositoryStateError::Watcher("repository snapshot worker is stopped".into())
            })
        }
    }

    struct TokioInvalidationReceiver(mpsc::UnboundedReceiver<InvalidateReason>);

    #[async_trait::async_trait]
    impl RepositoryStateInvalidationReceiver for TokioInvalidationReceiver {
        async fn recv(&mut self) -> Option<InvalidateReason> {
            self.0.recv().await
        }

        fn try_recv(&mut self) -> Option<InvalidateReason> {
            self.0.try_recv().ok()
        }
    }

    #[async_trait::async_trait]
    impl RepositoryStateWorkerRuntime for TestRepositoryStateWorkerRuntime {
        fn invalidation_channel(
            &self,
        ) -> (
            Box<dyn RepositoryStateInvalidationSender>,
            Box<dyn RepositoryStateInvalidationReceiver>,
        ) {
            let (tx, rx) = mpsc::unbounded_channel();
            (
                Box::new(TokioInvalidationSender(tx)),
                Box::new(TokioInvalidationReceiver(rx)),
            )
        }

        async fn scan(
            &self,
            scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            crate::common::operation_context::spawn_blocking(move || scanner.scan(&repo_path))
                .await
                .map_err(|err| RepositoryStateError::Watcher(format!("test scan failed: {err}")))?
        }

        async fn scan_worktrees(
            &self,
            scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<Vec<Worktree>, RepositoryStateError> {
            crate::common::operation_context::spawn_blocking(move || {
                scanner.scan_worktrees(&repo_path)
            })
            .await
            .map_err(|err| RepositoryStateError::Watcher(format!("test scan failed: {err}")))?
        }
    }

    /// tokio ランタイム外（std スレッド）から `WorktreeState::new` を呼ぶテスト用。
    /// worker を spawn しないため snapshot は更新されない。
    #[cfg(test)]
    pub(crate) struct NoSpawnRepositoryStateWorkerRuntime;

    #[cfg(test)]
    #[async_trait::async_trait]
    impl RepositoryStateWorkerRuntime for NoSpawnRepositoryStateWorkerRuntime {
        fn invalidation_channel(
            &self,
        ) -> (
            Box<dyn RepositoryStateInvalidationSender>,
            Box<dyn RepositoryStateInvalidationReceiver>,
        ) {
            let (tx, rx) = mpsc::unbounded_channel();
            (
                Box::new(TokioInvalidationSender(tx)),
                Box::new(TokioInvalidationReceiver(rx)),
            )
        }

        async fn scan(
            &self,
            _scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Err(RepositoryStateError::Watcher(format!(
                "no-spawn runtime does not scan {repo_path}"
            )))
        }

        async fn scan_worktrees(
            &self,
            _scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<Vec<Worktree>, RepositoryStateError> {
            Err(RepositoryStateError::Watcher(format!(
                "no-spawn runtime does not scan {repo_path}"
            )))
        }
    }

    pub struct IdentityWorktreePathNormalizer;

    impl WorktreePathNormalizer for IdentityWorktreePathNormalizer {
        fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
            Ok(PathBuf::from(worktree_path))
        }
    }
}
