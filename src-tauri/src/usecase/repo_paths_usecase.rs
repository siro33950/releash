//! repo_paths 責務のユースケース（リポジトリパス一覧の取得・追加・削除）。
//!
//! Repository パスの更新と購読への配信を順序付ける。
//! 「追加/削除が成功した時だけ現在の一覧 payload で通知する」という業務手順を
//! 注入された `RepoPathsNotifier`（送信手段の抽象）経由で実行し、controller は
//! ユースケースを呼ぶ薄い入口に徹する。

use std::sync::Arc;

use crate::domain::repository::{RepoPathsNotifier, RepoPathsRepository};

use super::repository_error::UsecaseError;

#[derive(Clone)]
pub struct RepoPathsUsecase {
    repo: Arc<dyn RepoPathsRepository>,
    notifier: Arc<dyn RepoPathsNotifier>,
    mutation: Arc<parking_lot::Mutex<()>>,
}

impl RepoPathsUsecase {
    pub fn new(repo: Arc<dyn RepoPathsRepository>, notifier: Arc<dyn RepoPathsNotifier>) -> Self {
        Self {
            repo,
            notifier,
            mutation: Arc::new(parking_lot::Mutex::new(())),
        }
    }

    pub fn get(&self) -> Vec<String> {
        self.repo.get()
    }

    /// 追加できた場合に `true`、既存・空文字で追加されなかった場合に `false`。
    /// 追加成功時のみ現在の一覧 payload で変更通知を発火する。
    pub fn add(&self, path: &str) -> Result<bool, UsecaseError> {
        let _mutation = self.mutation.lock();
        let added = self.repo.add(path)?;
        if added {
            self.notifier.notify_changed(self.repo.get());
        }
        Ok(added)
    }

    /// 削除できた場合に `true`、存在せず削除されなかった場合に `false`。
    /// 削除成功時のみ現在の一覧 payload で変更通知を発火する。
    pub fn remove(&self, path: &str) -> Result<bool, UsecaseError> {
        let _mutation = self.mutation.lock();
        let removed = self.repo.remove(path)?;
        if removed {
            self.notifier.notify_changed(self.repo.get());
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod repo_paths_usecase_tests {
    use super::*;
    use crate::domain::repository::RepositoryError;
    use parking_lot::Mutex;

    /// ドメイン抽象のみに依存することを検証するための手書き fake。
    #[derive(Default)]
    struct FakeRepoPaths {
        paths: Mutex<Vec<String>>,
        fail: bool,
    }

    impl RepoPathsRepository for FakeRepoPaths {
        fn get(&self) -> Vec<String> {
            self.paths.lock().clone()
        }
        fn add(&self, path: &str) -> Result<bool, RepositoryError> {
            if self.fail {
                return Err(RepositoryError::External("boom".to_string()));
            }
            let mut p = self.paths.lock();
            if p.iter().any(|x| x == path) {
                return Ok(false);
            }
            p.push(path.to_string());
            Ok(true)
        }
        fn remove(&self, path: &str) -> Result<bool, RepositoryError> {
            let mut p = self.paths.lock();
            let before = p.len();
            p.retain(|x| x != path);
            Ok(p.len() != before)
        }
    }

    /// 通知発火（payload と回数）を記録する fake。
    #[derive(Default)]
    struct FakeNotifier {
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl RepoPathsNotifier for FakeNotifier {
        fn notify_changed(&self, paths: Vec<String>) {
            self.calls.lock().push(paths);
        }
    }

    fn usecase_with(repo: Arc<FakeRepoPaths>, notifier: Arc<FakeNotifier>) -> RepoPathsUsecase {
        RepoPathsUsecase::new(repo, notifier)
    }

    #[test]
    fn test_追加_取得_削除を委譲する() {
        let uc = usecase_with(
            Arc::new(FakeRepoPaths::default()),
            Arc::new(FakeNotifier::default()),
        );

        assert!(uc.add("/repo/a").unwrap());
        assert!(!uc.add("/repo/a").unwrap()); // 重複は false
        assert_eq!(uc.get(), vec!["/repo/a".to_string()]);
        assert!(uc.remove("/repo/a").unwrap());
        assert!(uc.get().is_empty());
    }

    #[test]
    fn test_ドメインエラーをusecaseエラーへ変換する() {
        let uc = usecase_with(
            Arc::new(FakeRepoPaths {
                fail: true,
                ..Default::default()
            }),
            Arc::new(FakeNotifier::default()),
        );
        let err = uc.add("/repo/a").unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn test_追加削除成功時のみ現在の一覧payloadで通知する() {
        // behavior.md: リポジトリパス変更時に同じ変更通知が観測される。
        let repo = Arc::new(FakeRepoPaths::default());
        let notifier = Arc::new(FakeNotifier::default());
        let uc = usecase_with(repo, notifier.clone());

        // 追加成功 → 追加後の一覧 payload で 1 回通知。
        assert!(uc.add("/repo/a").unwrap());
        // 重複追加（false）→ 通知しない。
        assert!(!uc.add("/repo/a").unwrap());
        // 削除成功 → 削除後の一覧 payload で 1 回通知。
        assert!(uc.remove("/repo/a").unwrap());
        // 未存在削除（false）→ 通知しない。
        assert!(!uc.remove("/repo/a").unwrap());

        let calls = notifier.calls.lock();
        assert_eq!(
            *calls,
            vec![vec!["/repo/a".to_string()], Vec::<String>::new()],
            "成功時のみ、その時点の一覧 payload で通知される"
        );
    }
    #[test]
    fn test_並行追加削除_通知が完了するまで次の変更を開始しない() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        };
        struct BlockingNotifier {
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
            count: AtomicUsize,
            calls: Mutex<Vec<Vec<String>>>,
        }
        impl RepoPathsNotifier for BlockingNotifier {
            fn notify_changed(&self, paths: Vec<String>) {
                if self.count.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.entered.send(()).unwrap();
                    self.release.lock().recv().unwrap();
                }
                self.calls.lock().push(paths);
            }
        }
        for add_first in [true, false] {
            // Given
            let repo = Arc::new(FakeRepoPaths::default());
            if !add_first {
                repo.add("/repo").unwrap();
            }
            let (entered, waiting) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let notifier = Arc::new(BlockingNotifier {
                entered,
                release: Mutex::new(resume),
                count: AtomicUsize::new(0),
                calls: Mutex::new(Vec::new()),
            });
            let uc = Arc::new(RepoPathsUsecase::new(repo.clone(), notifier.clone()));
            // When
            std::thread::scope(|scope| {
                let first = scope.spawn(|| {
                    if add_first {
                        uc.add("/repo")
                    } else {
                        uc.remove("/repo")
                    }
                });
                waiting
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                let locked = uc.mutation.try_lock().is_none();
                let (started, ready) = mpsc::channel();
                let second_uc = uc.clone();
                let second = scope.spawn(move || {
                    started.send(()).unwrap();
                    if add_first {
                        second_uc.remove("/repo")
                    } else {
                        second_uc.add("/repo")
                    }
                });
                ready.recv().unwrap();
                release.send(()).unwrap();
                assert!(first.join().unwrap().unwrap());
                assert!(second.join().unwrap().unwrap());
                assert!(
                    locked,
                    "通知が完了する前にmutation lockを解放してはならない"
                );
            });
            // Then
            let present = vec!["/repo".to_string()];
            let expected = if add_first {
                vec![present, vec![]]
            } else {
                vec![vec![], present]
            };
            assert_eq!(*notifier.calls.lock(), expected);
            assert_eq!(repo.get(), *expected.last().unwrap());
        }
    }
}
