use super::super::{
    ProviderKind, ProviderLifecycleBinding, ProviderLifecycleCapabilityHash,
    ProviderLifecycleEvent, ProviderLifecycleOutcome, ProviderLifecycleRejection,
    ProviderLifecycleScope, ProviderLifecycleSignal, ProviderLifecycleSlot,
    ProviderLifecycleSlotId, ScopedProviderLifecycleEvent,
};

fn scope(session_id: &str, node_execution_id: &str, attempt: u32) -> ProviderLifecycleScope {
    ProviderLifecycleScope::new(
        session_id,
        "workflow-execution-1",
        node_execution_id,
        attempt,
    )
    .unwrap()
}

fn binding(provider: ProviderKind) -> ProviderLifecycleBinding {
    ProviderLifecycleBinding::arm(
        "binding-1",
        provider,
        scope("agent-session-1", "node-execution-1", 1),
    )
    .unwrap()
}

fn session_start(provider: ProviderKind) -> ProviderLifecycleSignal {
    ProviderLifecycleSignal::session_started(
        "binding-1",
        provider,
        scope("agent-session-1", "node-execution-1", 1),
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
    let replacement = ProviderLifecycleBinding::arm(
        "binding-2",
        ProviderKind::Codex,
        scope("agent-session-1", "node-execution-1", 1),
    )
    .unwrap();

    let events = slot.arm(replacement, capability(2));
    let stale = slot.receive(&capability(1), session_start(ProviderKind::Codex));

    assert_eq!(
        events,
        vec![
            ScopedProviderLifecycleEvent::new(
                scope("agent-session-1", "node-execution-1", 1),
                ProviderLifecycleEvent::BindingExpired {
                    binding_id: "binding-1".to_string(),
                },
            ),
            ScopedProviderLifecycleEvent::new(
                scope("agent-session-1", "node-execution-1", 1),
                ProviderLifecycleEvent::BindingArmed {
                    slot_id: "workflow-slot-1".to_string(),
                    binding_id: "binding-2".to_string(),
                    provider: ProviderKind::Codex,
                    scope: scope("agent-session-1", "node-execution-1", 1),
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
fn test_providerライフサイクル登録_同一実行系列の新attemptが旧bindingを失効させる() {
    let mut slot = ProviderLifecycleSlot::new(slot_id());
    let previous = ProviderLifecycleBinding::arm(
        "binding-previous",
        ProviderKind::Codex,
        scope("agent-session-previous", "node-execution-previous", 1),
    )
    .unwrap();
    slot.arm(previous, capability(1));
    let current = ProviderLifecycleBinding::arm(
        "binding-current",
        ProviderKind::Codex,
        scope("agent-session-current", "node-execution-current", 2),
    )
    .unwrap();

    slot.arm(current, capability(2));
    let stale = slot.receive(
        &capability(1),
        ProviderLifecycleSignal::session_started(
            "binding-previous",
            ProviderKind::Codex,
            scope("agent-session-previous", "node-execution-previous", 1),
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
