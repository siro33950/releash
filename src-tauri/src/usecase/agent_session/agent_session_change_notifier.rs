/// AgentSession の読み取りモデルに変化があったことを
/// クライアント surface へ通知する port。adaptor 側で実装する。
pub(crate) trait AgentSessionChangeNotifier: Send + Sync {
    fn agent_session_changed(&self, worktree_path: &str);
}
