use crate::domain::provider_lifecycle::ProviderKind;

use super::{
    ProviderAvailability, ProviderExecutable, ProviderRegistry, ProviderRegistryEntry,
    ProviderRegistryError, ProviderUnavailableReason, ResolvedProviderExecutable,
};

fn resolved(value: &str) -> ResolvedProviderExecutable {
    ResolvedProviderExecutable::new(std::path::PathBuf::from(value)).unwrap()
}

fn available(
    provider: ProviderKind,
    configured: Option<&str>,
    resolved: &str,
) -> ProviderRegistryEntry {
    ProviderRegistryEntry::detect(
        provider,
        configured.map(|value| ProviderExecutable::new(value).unwrap()),
        |_| ProviderAvailability::available(self::resolved(resolved)),
    )
}

fn unavailable(provider: ProviderKind, reason: ProviderUnavailableReason) -> ProviderRegistryEntry {
    ProviderRegistryEntry::detect(provider, None, |_| {
        ProviderAvailability::unavailable(reason)
    })
}

#[test]
fn test_provider_registry_availableはresolved_executableを必ず所有する() {
    let entry = available(
        ProviderKind::Claude,
        Some("/custom/claude"),
        "/custom/claude",
    );

    assert!(entry.is_available());
    assert_eq!(entry.effective_executable().as_str(), "/custom/claude");
    assert_eq!(
        entry.resolved_executable().map(|value| value.as_path()),
        Some(std::path::Path::new("/custom/claude"))
    );
    assert_eq!(entry.unavailable_reason(), None);
}

#[test]
fn test_provider_registry_unavailableはreasonを必ず所有しresolvedを持たない() {
    let entry = unavailable(
        ProviderKind::Codex,
        ProviderUnavailableReason::NotExecutable,
    );

    assert!(!entry.is_available());
    assert_eq!(entry.resolved_executable(), None);
    assert_eq!(
        entry.unavailable_reason(),
        Some(ProviderUnavailableReason::NotExecutable)
    );
}

#[test]
fn test_provider_registry_overrideとresetでeffective_executableを切り替える() {
    let overridden = available(
        ProviderKind::Claude,
        Some("/custom/claude"),
        "/custom/claude",
    );
    assert_eq!(overridden.effective_executable().as_str(), "/custom/claude");

    let reset = available(ProviderKind::Claude, None, "/path/claude");
    assert_eq!(reset.effective_executable().as_str(), "claude");
    assert_eq!(reset.configured_executable(), None);
}

#[test]
fn test_provider_registry_snapshotは全supported_providerを一括で要求する() {
    let complete = ProviderRegistry::new(vec![
        available(ProviderKind::Claude, None, "/path/claude"),
        unavailable(ProviderKind::Codex, ProviderUnavailableReason::NotFound),
    ])
    .unwrap();
    assert_eq!(complete.entries().len(), 2);

    assert_eq!(
        ProviderRegistry::new(vec![available(ProviderKind::Claude, None, "/path/claude")]),
        Err(ProviderRegistryError::Incomplete)
    );
    assert_eq!(
        ProviderRegistry::new(vec![
            available(ProviderKind::Claude, None, "/path/claude"),
            available(ProviderKind::Claude, None, "/other/claude"),
        ]),
        Err(ProviderRegistryError::DuplicateProvider)
    );
}

#[test]
fn test_provider_registry_executableは空文字を拒否する() {
    assert_eq!(
        ProviderExecutable::new("  "),
        Err(ProviderRegistryError::InvalidExecutable)
    );
}

#[test]
fn test_provider_registry_entryはeffective_executableをprobeへ渡して判定を適用する() {
    let configured = ProviderExecutable::new("/custom/claude").unwrap();

    let entry =
        ProviderRegistryEntry::detect(ProviderKind::Claude, Some(configured), |effective| {
            assert_eq!(effective.as_str(), "/custom/claude");
            ProviderAvailability::available(resolved("/resolved/claude"))
        });

    assert_eq!(entry.effective_executable().as_str(), "/custom/claude");
    assert_eq!(
        entry.resolved_executable().map(|value| value.as_path()),
        Some(std::path::Path::new("/resolved/claude"))
    );
}
