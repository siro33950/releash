use super::super::{
    ProviderKind, ProviderLifecycleBinding, ProviderLifecycleEvent, ProviderLifecycleOutcome,
    ProviderLifecycleRejection, ProviderLifecycleReplayError, ProviderLifecycleScope,
    ProviderLifecycleSignal, ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation,
    ProviderLifecycleUnavailableReason,
};
use crate::domain::workflow::AgentSessionActivity;

fn scope(session_id: &str) -> ProviderLifecycleScope {
    ProviderLifecycleScope::new(session_id).unwrap()
}

fn binding(provider: ProviderKind) -> ProviderLifecycleBinding {
    ProviderLifecycleBinding::arm("binding-1", provider, scope("agent-session-1")).unwrap()
}

fn session_start(provider: ProviderKind) -> ProviderLifecycleSignal {
    ProviderLifecycleSignal::session_started(
        "binding-1",
        provider,
        scope("agent-session-1"),
        "provider-session-1",
        Some("provider://transcript/1"),
    )
    .unwrap()
}

fn stop(provider: ProviderKind) -> ProviderLifecycleSignal {
    ProviderLifecycleSignal::stop_observed(
        "binding-1",
        provider,
        scope("agent-session-1"),
        "provider-session-1",
        Some("provider://transcript/1"),
    )
    .unwrap()
}

fn activity(provider: ProviderKind, activity: AgentSessionActivity) -> ProviderLifecycleSignal {
    ProviderLifecycleSignal::activity_observed(
        "binding-1",
        provider,
        scope("agent-session-1"),
        "provider-session-1",
        Some("provider://transcript/1"),
        activity,
    )
    .unwrap()
}

fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("workflow-slot-1").unwrap()
}

fn unavailable(
    provider: ProviderKind,
    candidate_scope: ProviderLifecycleScope,
) -> ProviderLifecycleUnavailableObservation {
    ProviderLifecycleUnavailableObservation::new(
        "binding-1",
        provider,
        candidate_scope,
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
    )
    .unwrap()
}

#[test]
fn test_providerライフサイクル利用不能_同一観測の再送は診断事実を重複させない() {
    let mut binding = binding(ProviderKind::Codex);
    let observation = unavailable(ProviderKind::Codex, scope("agent-session-1"));

    assert!(matches!(
        binding.mark_unavailable(observation.clone()),
        ProviderLifecycleOutcome::Applied(_)
    ));
    assert_eq!(
        binding.mark_unavailable(observation),
        ProviderLifecycleOutcome::Duplicate
    );
}

#[test]
fn test_providerライフサイクル利用不能_異なる後続観測へ診断を更新する() {
    let mut binding = binding(ProviderKind::Codex);
    assert!(matches!(
        binding.mark_unavailable(unavailable(ProviderKind::Codex, scope("agent-session-1"),)),
        ProviderLifecycleOutcome::Applied(_)
    ));
    let updated = ProviderLifecycleUnavailableObservation::new(
        "binding-1",
        ProviderKind::Codex,
        scope("agent-session-1"),
        ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    )
    .unwrap();

    assert_eq!(
        binding.mark_unavailable(updated),
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id: "binding-1".to_string(),
            provider: ProviderKind::Codex,
            scope: scope("agent-session-1"),
            reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        },])
    );
}

#[test]
fn test_providerライフサイクル利用不能_scope不一致と失効済みbindingを拒否する() {
    let mut binding = binding(ProviderKind::Codex);
    let wrong_scope = unavailable(ProviderKind::Codex, scope("other-agent-session"));

    assert_eq!(
        binding.mark_unavailable(wrong_scope),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ScopeMismatch)
    );
    assert!(matches!(
        binding.expire(),
        ProviderLifecycleOutcome::Applied(_)
    ));
    assert_eq!(
        binding.mark_unavailable(unavailable(ProviderKind::Codex, scope("agent-session-1"),)),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
}

#[test]
fn test_providerライフサイクル利用不能_後続の正常session_startで回復する() {
    let mut binding = binding(ProviderKind::Codex);
    assert!(matches!(
        binding.mark_unavailable(unavailable(ProviderKind::Codex, scope("agent-session-1"),)),
        ProviderLifecycleOutcome::Applied(_)
    ));

    assert_eq!(
        binding.observe(session_start(ProviderKind::Codex)),
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::SessionAssociated {
            binding_id: "binding-1".to_string(),
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: Some("provider://transcript/1".to_string()),
        }])
    );
    assert_eq!(binding.provider_session_id(), Some("provider-session-1"));
}

#[test]
fn test_providerライフサイクル利用不能_session_start受理後の期限切れ報告を拒否する() {
    let mut binding = binding(ProviderKind::Codex);
    assert!(matches!(
        binding.observe(session_start(ProviderKind::Codex)),
        ProviderLifecycleOutcome::Applied(_)
    ));

    assert_eq!(
        binding.mark_unavailable(unavailable(ProviderKind::Codex, scope("agent-session-1"),)),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::SessionAlreadyAssociated)
    );
}

#[test]
fn test_providerライフサイクル受信_最初のsession_startをscopeへ関連付ける() {
    let mut binding = binding(ProviderKind::Codex);

    let outcome = binding.observe(session_start(ProviderKind::Codex));

    assert_eq!(
        outcome,
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::SessionAssociated {
            binding_id: "binding-1".to_string(),
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: Some("provider://transcript/1".to_string()),
        }])
    );
    assert_eq!(binding.provider_session_id(), Some("provider-session-1"));
    assert_eq!(binding.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_providerライフサイクル受信_同一session_startの再送は冪等になる() {
    let mut binding = binding(ProviderKind::Claude);
    let signal = session_start(ProviderKind::Claude);
    assert!(matches!(
        binding.observe(signal.clone()),
        ProviderLifecycleOutcome::Applied(_)
    ));

    let duplicate = binding.observe(signal);

    assert_eq!(duplicate, ProviderLifecycleOutcome::Duplicate);
    assert_eq!(binding.provider_session_id(), Some("provider-session-1"));
}

#[test]
fn test_providerライフサイクル受信_異なるprovider_sessionへの差し替えを拒否する() {
    let mut binding = binding(ProviderKind::Codex);
    binding.observe(session_start(ProviderKind::Codex));
    let conflicting = ProviderLifecycleSignal::session_started(
        "binding-1",
        ProviderKind::Codex,
        scope("agent-session-1"),
        "provider-session-2",
        Some("provider://transcript/2"),
    )
    .unwrap();

    let outcome = binding.observe(conflicting);

    assert_eq!(
        outcome,
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ProviderSessionMismatch)
    );
    assert_eq!(binding.provider_session_id(), Some("provider-session-1"));
    assert_eq!(binding.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_providerライフサイクル受信_同一agent_sessionの後続stopも観測事実として受理する() {
    let mut binding = binding(ProviderKind::Codex);
    assert_eq!(
        binding.observe(stop(ProviderKind::Codex)),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::SessionNotAssociated)
    );
    binding.observe(session_start(ProviderKind::Codex));

    let accepted = binding.observe(stop(ProviderKind::Codex));
    let next_turn = binding.observe(stop(ProviderKind::Codex));

    assert_eq!(
        accepted,
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::StopObserved {
            binding_id: "binding-1".to_string(),
        }])
    );
    assert_eq!(
        next_turn,
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::StopObserved {
            binding_id: "binding-1".to_string(),
        }])
    );
}

#[test]
fn test_providerライフサイクル受信_活動観測はsessionを検証してlifecycle事実を作らない() {
    let mut binding = binding(ProviderKind::Codex);
    assert_eq!(
        binding.observe(activity(ProviderKind::Codex, AgentSessionActivity::Working,)),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::SessionNotAssociated)
    );
    binding.observe(session_start(ProviderKind::Codex));

    for observed in [
        AgentSessionActivity::Working,
        AgentSessionActivity::AwaitingAnswer,
        AgentSessionActivity::AwaitingInstruction,
    ] {
        assert_eq!(
            binding.observe(activity(ProviderKind::Codex, observed)),
            ProviderLifecycleOutcome::Duplicate
        );
    }
    assert_eq!(binding.provider_session_id(), Some("provider-session-1"));
    assert_eq!(binding.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_providerライフサイクル受信_活動観測は未設定transcriptだけを関連付ける() {
    let mut binding = binding(ProviderKind::Claude);
    binding.observe(
        ProviderLifecycleSignal::session_started(
            "binding-1",
            ProviderKind::Claude,
            scope("agent-session-1"),
            "provider-session-1",
            None,
        )
        .unwrap(),
    );

    let outcome = binding.observe(activity(
        ProviderKind::Claude,
        AgentSessionActivity::Working,
    ));

    assert_eq!(
        outcome,
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::TranscriptAssociated {
            binding_id: "binding-1".to_string(),
            transcript_ref: "provider://transcript/1".to_string(),
        }])
    );
    assert_eq!(binding.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_providerライフサイクル受信_活動観測もbinding_provider_scope_session不一致を拒否する() {
    let mut binding = binding(ProviderKind::Claude);
    binding.observe(session_start(ProviderKind::Claude));
    let signal = |binding_id: &str,
                  provider: ProviderKind,
                  candidate_scope: ProviderLifecycleScope,
                  provider_session_id: &str| {
        ProviderLifecycleSignal::activity_observed(
            binding_id,
            provider,
            candidate_scope,
            provider_session_id,
            Some("provider://transcript/1"),
            AgentSessionActivity::Working,
        )
        .unwrap()
    };

    assert_eq!(
        binding.observe(signal(
            "binding-2",
            ProviderKind::Claude,
            scope("agent-session-1"),
            "provider-session-1",
        )),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingMismatch)
    );
    assert_eq!(
        binding.observe(signal(
            "binding-1",
            ProviderKind::Codex,
            scope("agent-session-1"),
            "provider-session-1",
        )),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ProviderMismatch)
    );
    assert_eq!(
        binding.observe(signal(
            "binding-1",
            ProviderKind::Claude,
            scope("agent-session-2"),
            "provider-session-1",
        )),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ScopeMismatch)
    );
    assert_eq!(
        binding.observe(signal(
            "binding-1",
            ProviderKind::Claude,
            scope("agent-session-1"),
            "provider-session-2",
        )),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ProviderSessionMismatch)
    );

    binding.expire();
    assert_eq!(
        binding.observe(activity(
            ProviderKind::Claude,
            AgentSessionActivity::Working,
        )),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
}

#[test]
fn test_providerライフサイクル受信_bindingとproviderとscopeの不一致を拒否する() {
    let mut binding = binding(ProviderKind::Claude);
    let wrong_binding = ProviderLifecycleSignal::session_started(
        "binding-2",
        ProviderKind::Claude,
        scope("agent-session-1"),
        "provider-session-1",
        None,
    )
    .unwrap();
    let wrong_provider = ProviderLifecycleSignal::session_started(
        "binding-1",
        ProviderKind::Codex,
        scope("agent-session-1"),
        "provider-session-1",
        None,
    )
    .unwrap();
    let wrong_scope = ProviderLifecycleSignal::session_started(
        "binding-1",
        ProviderKind::Claude,
        scope("agent-session-2"),
        "provider-session-1",
        None,
    )
    .unwrap();

    assert_eq!(
        binding.observe(wrong_binding),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingMismatch)
    );
    assert_eq!(
        binding.observe(wrong_provider),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ProviderMismatch)
    );
    assert_eq!(
        binding.observe(wrong_scope),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::ScopeMismatch)
    );
    assert_eq!(binding.provider_session_id(), None);
}

#[test]
fn test_providerライフサイクル受信_失効後の信号を拒否する() {
    let mut expired = binding(ProviderKind::Codex);
    assert_eq!(
        expired.expire(),
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::BindingExpired {
            binding_id: "binding-1".to_string(),
        }])
    );
    assert_eq!(
        expired.observe(session_start(ProviderKind::Codex)),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
}

#[test]
fn test_providerライフサイクル受信_stop_failureはstopへ変換しない() {
    let mut binding = binding(ProviderKind::Claude);
    binding.observe(session_start(ProviderKind::Claude));
    let stop_failure = ProviderLifecycleSignal::stop_failed(
        "binding-1",
        ProviderKind::Claude,
        scope("agent-session-1"),
        "provider-session-1",
        Some("provider://transcript/1"),
        "provider request failed",
    )
    .unwrap();

    let outcome = binding.observe(stop_failure);

    assert_eq!(
        outcome,
        ProviderLifecycleOutcome::Applied(vec![ProviderLifecycleEvent::StopFailed {
            binding_id: "binding-1".to_string(),
            reason: "provider request failed".to_string(),
        }])
    );
}

#[test]
fn test_providerライフサイクル受信_null_transcriptは後から補完でき確定後の差し替えは拒否する() {
    let mut binding = binding(ProviderKind::Codex);
    let without_transcript = ProviderLifecycleSignal::session_started(
        "binding-1",
        ProviderKind::Codex,
        scope("agent-session-1"),
        "provider-session-1",
        None,
    )
    .unwrap();
    binding.observe(without_transcript);
    let with_transcript = stop(ProviderKind::Codex);

    let accepted = binding.observe(with_transcript);

    assert_eq!(
        accepted,
        ProviderLifecycleOutcome::Applied(vec![
            ProviderLifecycleEvent::TranscriptAssociated {
                binding_id: "binding-1".to_string(),
                transcript_ref: "provider://transcript/1".to_string(),
            },
            ProviderLifecycleEvent::StopObserved {
                binding_id: "binding-1".to_string(),
            },
        ])
    );
    assert_eq!(binding.transcript_ref(), Some("provider://transcript/1"));

    let conflicting = ProviderLifecycleSignal::stop_observed(
        "binding-1",
        ProviderKind::Codex,
        scope("agent-session-1"),
        "provider-session-1",
        Some("provider://transcript/2"),
    )
    .unwrap();
    assert_eq!(
        binding.observe(conflicting),
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::TranscriptMismatch)
    );
}

#[test]
fn test_providerライフサイクル再生_durable_event列から同じbindingを復元する() {
    let mut original = binding(ProviderKind::Codex);
    let mut events = vec![original.armed_event(&slot_id())];
    let ProviderLifecycleOutcome::Applied(started) =
        original.observe(session_start(ProviderKind::Codex))
    else {
        panic!("session start must be applied");
    };
    events.extend(started);
    let ProviderLifecycleOutcome::Applied(stopped) = original.observe(stop(ProviderKind::Codex))
    else {
        panic!("stop must be applied");
    };
    events.extend(stopped);

    let restored = ProviderLifecycleBinding::rehydrate(events).unwrap();

    assert_eq!(restored.binding_id(), "binding-1");
    assert_eq!(restored.provider(), ProviderKind::Codex);
    assert_eq!(restored.scope(), &scope("agent-session-1"));
    assert_eq!(restored.provider_session_id(), Some("provider-session-1"));
    assert_eq!(restored.transcript_ref(), Some("provider://transcript/1"));
}

#[test]
fn test_providerライフサイクル再生_利用不能後のsession関連付けで回復状態を復元する() {
    let events = vec![
        binding(ProviderKind::Codex).armed_event(&slot_id()),
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id: "binding-1".to_string(),
            provider: ProviderKind::Codex,
            scope: scope("agent-session-1"),
            reason: ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
        },
        ProviderLifecycleEvent::SessionAssociated {
            binding_id: "binding-1".to_string(),
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: None,
        },
    ];

    let restored = ProviderLifecycleBinding::rehydrate(events).unwrap();

    assert_eq!(restored.provider_session_id(), Some("provider-session-1"));
}

#[test]
fn test_providerライフサイクル再生_session関連付け後の利用不能を拒否する() {
    let events = vec![
        binding(ProviderKind::Codex).armed_event(&slot_id()),
        ProviderLifecycleEvent::SessionAssociated {
            binding_id: "binding-1".to_string(),
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: None,
        },
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id: "binding-1".to_string(),
            provider: ProviderKind::Codex,
            scope: scope("agent-session-1"),
            reason: ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
        },
    ];

    assert_eq!(
        ProviderLifecycleBinding::rehydrate(events),
        Err(ProviderLifecycleReplayError::InvalidTransition)
    );
}
