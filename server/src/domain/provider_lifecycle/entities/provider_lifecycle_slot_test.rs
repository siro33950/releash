use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleBinding, ProviderLifecycleCapabilityHash,
    ProviderLifecycleEvent, ProviderLifecycleOutcome, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSlot,
    ProviderLifecycleSlotId, ScopedProviderLifecycleEvent,
};

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

fn capability(byte: u8) -> ProviderLifecycleCapabilityHash {
    ProviderLifecycleCapabilityHash::from_digest([byte; 32])
}

fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("workflow-slot-1").unwrap()
}

#[test]
fn test_providerライフサイクル登録_capability不一致の信号を拒否する() {
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    slot.arm(binding(ProviderKind::Codex), capability(1));

    let outcome = slot.receive(&capability(2), session_start(ProviderKind::Codex));

    assert_eq!(
        outcome,
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::InvalidCapability)
    );
}

#[test]
fn test_providerライフサイクルslot_新bindingが旧bindingを失効させる() {
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    slot.arm(binding(ProviderKind::Codex), capability(1));
    let replacement =
        ProviderLifecycleBinding::arm("binding-2", ProviderKind::Codex, scope("agent-session-1"))
            .unwrap();

    let events = slot.arm(replacement, capability(2));
    let stale = slot.receive(&capability(1), session_start(ProviderKind::Codex));

    assert_eq!(
        events,
        vec![
            ScopedProviderLifecycleEvent::new(
                scope("agent-session-1"),
                ProviderLifecycleEvent::BindingExpired {
                    binding_id: "binding-1".to_string(),
                },
            ),
            ScopedProviderLifecycleEvent::new(
                scope("agent-session-1"),
                ProviderLifecycleEvent::BindingArmed {
                    slot_id: "workflow-slot-1".to_string(),
                    binding_id: "binding-2".to_string(),
                    provider: ProviderKind::Codex,
                    scope: scope("agent-session-1"),
                },
            ),
        ]
    );
    assert_eq!(
        stale,
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
}

#[test]
fn test_providerライフサイクル登録_同一slotの再起動が旧bindingを失効させる() {
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    let previous = ProviderLifecycleBinding::arm(
        "binding-previous",
        ProviderKind::Codex,
        scope("agent-session-previous"),
    )
    .unwrap();
    slot.arm(previous, capability(1));
    let current = ProviderLifecycleBinding::arm(
        "binding-current",
        ProviderKind::Codex,
        scope("agent-session-current"),
    )
    .unwrap();

    slot.arm(current, capability(2));
    let stale = slot.receive(
        &capability(1),
        ProviderLifecycleSignal::session_started(
            "binding-previous",
            ProviderKind::Codex,
            scope("agent-session-previous"),
            "provider-session-previous",
            None,
        )
        .unwrap(),
    );

    assert_eq!(
        stale,
        ProviderLifecycleOutcome::Rejected(ProviderLifecycleRejection::BindingExpired)
    );
}

#[test]
fn test_provider_session参照_開始観測済みの現行bindingだけを返す() {
    // Given
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    slot.arm(binding(ProviderKind::Codex), capability(1));
    assert!(slot.current_provider_session(ProviderKind::Codex).is_none());
    // When
    slot.receive(&capability(1), session_start(ProviderKind::Codex));
    // Then
    assert_eq!(
        slot.current_provider_session(ProviderKind::Codex),
        Some("provider-session-1")
    );
    assert!(slot
        .current_provider_session(ProviderKind::Claude)
        .is_none());
    slot.release("binding-1");
    assert!(slot.current_provider_session(ProviderKind::Codex).is_none());
}

#[test]
fn test_使用率資格_slotとbindingとproviderとsessionとcapabilityを照合する() {
    // Given
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    assert!(slot
        .authorize_usage(
            &capability(1),
            "binding-1",
            ProviderKind::Claude,
            &scope("agent-session-1")
        )
        .is_err());
    slot.arm(binding(ProviderKind::Claude), capability(1));
    // When / Then
    assert!(slot
        .authorize_usage(
            &capability(1),
            "binding-1",
            ProviderKind::Claude,
            &scope("agent-session-1")
        )
        .is_ok());
    for (hash, id, provider, session) in [
        (
            capability(2),
            "binding-1",
            ProviderKind::Claude,
            "agent-session-1",
        ),
        (
            capability(1),
            "stale",
            ProviderKind::Claude,
            "agent-session-1",
        ),
        (
            capability(1),
            "binding-1",
            ProviderKind::Codex,
            "agent-session-1",
        ),
        (capability(1), "binding-1", ProviderKind::Claude, "other"),
    ] {
        assert!(slot
            .authorize_usage(&hash, id, provider, &scope(session))
            .is_err());
    }
    slot.release("binding-1");
    assert!(slot
        .authorize_usage(
            &capability(1),
            "binding-1",
            ProviderKind::Claude,
            &scope("agent-session-1")
        )
        .is_err());
}
