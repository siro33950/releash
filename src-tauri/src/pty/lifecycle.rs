use std::path::Path;
use std::time::{Duration, Instant};

use super::backend::{ExistingSession, PtyBackend};

#[allow(dead_code)]
const ORPHAN_CHECK_INTERVAL: Duration = Duration::from_secs(300); // 5 minutes
#[allow(dead_code)]
const SESSION_PREFIX: &str = "releash";

#[allow(dead_code)]
pub struct SessionLifecycle {
    last_check: Option<Instant>,
}

impl SessionLifecycle {
    pub fn new() -> Self {
        Self { last_check: None }
    }

    #[allow(dead_code)]
    pub fn scan_orphans(
        &mut self,
        backend: &dyn PtyBackend,
    ) -> Result<Vec<ExistingSession>, String> {
        self.last_check = Some(Instant::now());

        let sessions = backend.list_existing()?;
        let orphans: Vec<ExistingSession> = sessions
            .into_iter()
            .filter(|s| s.session_id.starts_with(SESSION_PREFIX))
            .filter(|s| {
                // A session is orphaned if its worktree path no longer exists
                match &s.worktree_path {
                    Some(path) => !Path::new(path).exists(),
                    None => false, // Can't determine orphan status without path
                }
            })
            .collect();

        Ok(orphans)
    }

    #[allow(dead_code)]
    pub fn should_check(&self) -> bool {
        match self.last_check {
            None => true,
            Some(last) => last.elapsed() >= ORPHAN_CHECK_INTERVAL,
        }
    }

    #[allow(dead_code)]
    pub fn find_restorable(
        &self,
        backend: &dyn PtyBackend,
    ) -> Result<Vec<ExistingSession>, String> {
        let sessions = backend.list_existing()?;
        let restorable: Vec<ExistingSession> = sessions
            .into_iter()
            .filter(|s| s.session_id.starts_with(SESSION_PREFIX))
            .filter(|s| {
                match &s.worktree_path {
                    Some(path) => Path::new(path).exists(),
                    None => true, // Keep sessions without known path
                }
            })
            .collect();

        Ok(restorable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockBackend {
        sessions: Vec<ExistingSession>,
    }

    impl PtyBackend for MockBackend {
        fn spawn(
            &self,
            _config: super::super::backend::SpawnConfig,
        ) -> Result<super::super::backend::BackendSession, String> {
            Err("MockBackend does not support spawn".to_string())
        }

        fn attach(
            &self,
            _session_id: &str,
        ) -> Result<super::super::backend::BackendSession, String> {
            Err("MockBackend does not support attach".to_string())
        }

        fn list_existing(&self) -> Result<Vec<ExistingSession>, String> {
            Ok(self.sessions.clone())
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }
    }

    impl Clone for ExistingSession {
        fn clone(&self) -> Self {
            Self {
                session_id: self.session_id.clone(),
                worktree_path: self.worktree_path.clone(),
                label: self.label.clone(),
            }
        }
    }

    #[test]
    fn test_should_check_initially_true() {
        let lifecycle = SessionLifecycle::new();
        assert!(lifecycle.should_check());
    }

    #[test]
    fn test_should_check_false_after_scan() {
        let mut lifecycle = SessionLifecycle::new();
        let backend = MockBackend { sessions: vec![] };
        let _ = lifecycle.scan_orphans(&backend);
        assert!(!lifecycle.should_check());
    }

    #[test]
    fn test_scan_orphans_with_existing_path() {
        let mut lifecycle = SessionLifecycle::new();
        let backend = MockBackend {
            sessions: vec![ExistingSession {
                session_id: "releash-abc-def".to_string(),
                worktree_path: Some("/".to_string()), // Root always exists
                label: None,
            }],
        };
        let orphans = lifecycle.scan_orphans(&backend).unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_scan_orphans_with_nonexistent_path() {
        let mut lifecycle = SessionLifecycle::new();
        let backend = MockBackend {
            sessions: vec![ExistingSession {
                session_id: "releash-abc-def".to_string(),
                worktree_path: Some("/nonexistent/path/that/does/not/exist".to_string()),
                label: None,
            }],
        };
        let orphans = lifecycle.scan_orphans(&backend).unwrap();
        assert_eq!(orphans.len(), 1);
    }

    #[test]
    fn test_scan_orphans_ignores_non_releash_sessions() {
        let mut lifecycle = SessionLifecycle::new();
        let backend = MockBackend {
            sessions: vec![ExistingSession {
                session_id: "other-session".to_string(),
                worktree_path: Some("/nonexistent/path".to_string()),
                label: None,
            }],
        };
        let orphans = lifecycle.scan_orphans(&backend).unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_find_restorable_with_existing_path() {
        let lifecycle = SessionLifecycle::new();
        let backend = MockBackend {
            sessions: vec![
                ExistingSession {
                    session_id: "releash-abc-def".to_string(),
                    worktree_path: Some("/".to_string()),
                    label: None,
                },
                ExistingSession {
                    session_id: "releash-ghi-jkl".to_string(),
                    worktree_path: Some("/nonexistent/path".to_string()),
                    label: None,
                },
            ],
        };
        let restorable = lifecycle.find_restorable(&backend).unwrap();
        assert_eq!(restorable.len(), 1);
        assert_eq!(restorable[0].session_id, "releash-abc-def");
    }

    #[test]
    fn test_find_restorable_keeps_sessions_without_path() {
        let lifecycle = SessionLifecycle::new();
        let backend = MockBackend {
            sessions: vec![ExistingSession {
                session_id: "releash-abc-def".to_string(),
                worktree_path: None,
                label: None,
            }],
        };
        let restorable = lifecycle.find_restorable(&backend).unwrap();
        assert_eq!(restorable.len(), 1);
    }
}
