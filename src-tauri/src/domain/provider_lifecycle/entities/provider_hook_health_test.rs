use super::{ProviderHookHealth, ProviderHookHealthEvent, ProviderHookHealthOutcome};
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};

#[test]
fn test_provider_hook_health_異常をprovider別の最新warningとして保持する() {
    let mut health = ProviderHookHealth::new(ProviderKind::Codex);

    assert_eq!(health.warning(), None);
    health.observe_launch("launch-1");
    assert_eq!(
        health.observe_unavailable(
            "launch-1",
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        ),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::WarningRecorded {
            provider: ProviderKind::Codex,
            launch_id: "launch-1".to_string(),
            reason: ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        })
    );
    assert_eq!(
        health.warning(),
        Some((
            "launch-1",
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
        ))
    );
}

#[test]
fn test_provider_hook_health_同一異常を冪等にし新しいlaunchの異常へ更新する() {
    let mut health = ProviderHookHealth::new(ProviderKind::Claude);
    health.observe_launch("launch-1");
    health.observe_unavailable(
        "launch-1",
        ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    );

    assert_eq!(
        health.observe_unavailable(
            "launch-1",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        ),
        ProviderHookHealthOutcome::Duplicate
    );
    health.observe_launch("launch-2");
    assert!(matches!(
        health.observe_unavailable(
            "launch-2",
            ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected,
        ),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::WarningRecorded {
            launch_id,
            reason: ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected,
            ..
        }) if launch_id == "launch-2"
    ));
}

#[test]
fn test_provider_hook_health_新しいlaunchだけでは以前のwarningを解除しない() {
    let mut health = ProviderHookHealth::new(ProviderKind::Claude);
    health.observe_launch("launch-1");
    health.observe_unavailable(
        "launch-1",
        ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    );

    health.observe_launch("launch-2");

    assert_eq!(
        health.warning(),
        Some((
            "launch-1",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        ))
    );
}

#[test]
fn test_provider_hook_health_rehydrateでも新しいlaunchだけでは以前のwarningを解除しない() {
    let health = ProviderHookHealth::rehydrate(
        ProviderKind::Claude,
        &[
            ProviderHookHealthEvent::LaunchObserved {
                provider: ProviderKind::Claude,
                launch_id: "launch-1".to_string(),
            },
            ProviderHookHealthEvent::WarningRecorded {
                provider: ProviderKind::Claude,
                launch_id: "launch-1".to_string(),
                reason: ProviderLifecycleUnavailableReason::LocalApiUnavailable,
            },
            ProviderHookHealthEvent::LaunchObserved {
                provider: ProviderKind::Claude,
                launch_id: "launch-2".to_string(),
            },
        ],
    )
    .unwrap();

    assert_eq!(
        health.warning(),
        Some((
            "launch-1",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        ))
    );
}

#[test]
fn test_provider_hook_health_後続launchの正常session_startでwarningを解除する() {
    let mut health = ProviderHookHealth::new(ProviderKind::Claude);
    health.observe_launch("launch-1");
    health.observe_unavailable(
        "launch-1",
        ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
    );

    health.observe_launch("launch-2");
    assert_eq!(
        health.observe_session_started("launch-2"),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::SessionStartedObserved {
            provider: ProviderKind::Claude,
            launch_id: "launch-2".to_string(),
        })
    );
    assert_eq!(health.warning(), None);
    assert_eq!(
        health.observe_session_started("launch-2"),
        ProviderHookHealthOutcome::Duplicate
    );
}

#[test]
fn test_provider_hook_health_lifecycleで検証済みのsession_startはlaunch永続化より先でも適用する() {
    let mut health = ProviderHookHealth::new(ProviderKind::Codex);
    health.observe_launch("launch-1");
    health.observe_unavailable(
        "launch-1",
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
    );
    health.take_uncommitted_events();

    assert!(matches!(
        health.observe_active_session_started("launch-2"),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::SessionStartedObserved {
            launch_id,
            ..
        }) if launch_id == "launch-2"
    ));
    assert_eq!(health.latest_launch_id(), Some("launch-2"));
    assert!(health.latest_launch_session_started());
    assert_eq!(health.warning(), None);
    assert_eq!(health.take_uncommitted_events().len(), 2);
}

#[test]
fn test_provider_hook_health_古いlaunchの遅延session_startで最新warningを解除しない() {
    let mut health = ProviderHookHealth::new(ProviderKind::Codex);
    assert!(matches!(
        health.observe_launch("launch-1"),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::LaunchObserved { .. })
    ));
    health.observe_unavailable(
        "launch-1",
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
    );
    health.observe_launch("launch-2");
    health.observe_unavailable(
        "launch-2",
        ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    );

    assert_eq!(
        health.observe_session_started("launch-1"),
        ProviderHookHealthOutcome::Duplicate
    );
    assert_eq!(
        health.warning(),
        Some((
            "launch-2",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable
        ))
    );
    assert!(matches!(
        health.observe_session_started("launch-2"),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::SessionStartedObserved { .. })
    ));
}

#[test]
fn test_provider_hook_health_正常session_start後は同じlaunchを欠落扱いしない() {
    let mut health = ProviderHookHealth::new(ProviderKind::Claude);
    health.observe_launch("launch-1");

    assert!(matches!(
        health.observe_session_started("launch-1"),
        ProviderHookHealthOutcome::Applied(_)
    ));
    assert_eq!(
        health.observe_unavailable(
            "launch-1",
            ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded,
        ),
        ProviderHookHealthOutcome::Duplicate
    );
    assert_eq!(health.warning(), None);
}

#[test]
fn test_provider_hook_health_session_start後の配送失敗を警告し後続成功で解除する() {
    let mut health = ProviderHookHealth::new(ProviderKind::Claude);
    health.observe_launch("launch-1");
    health.observe_session_started("launch-1");

    assert!(matches!(
        health.observe_unavailable(
            "launch-1",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
        ),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::WarningRecorded { .. })
    ));
    assert_eq!(
        health.warning(),
        Some((
            "launch-1",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable
        ))
    );
    assert!(matches!(
        health.observe_session_started("launch-1"),
        ProviderHookHealthOutcome::Applied(ProviderHookHealthEvent::SessionStartedObserved { .. })
    ));
    assert_eq!(health.warning(), None);
}
