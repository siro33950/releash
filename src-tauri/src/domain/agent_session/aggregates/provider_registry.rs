use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderExecutable(String);

impl ProviderExecutable {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, ProviderRegistryError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.contains('\0') {
            return Err(ProviderRegistryError::InvalidExecutable);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedProviderExecutable(PathBuf);

impl ResolvedProviderExecutable {
    pub(crate) fn new(path: PathBuf) -> Result<Self, ProviderRegistryError> {
        if path.as_os_str().is_empty() || !path.is_absolute() {
            return Err(ProviderRegistryError::InvalidResolvedExecutable);
        }
        Ok(Self(path))
    }

    pub(crate) fn as_path(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderUnavailableReason {
    NotFound,
    NotExecutable,
    SearchPathUnavailable,
    ProbeFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderAvailability {
    Available {
        resolved_executable: ResolvedProviderExecutable,
    },
    Unavailable {
        reason: ProviderUnavailableReason,
    },
}

impl ProviderAvailability {
    pub(crate) fn available(resolved_executable: ResolvedProviderExecutable) -> Self {
        Self::Available {
            resolved_executable,
        }
    }

    pub(crate) fn unavailable(reason: ProviderUnavailableReason) -> Self {
        Self::Unavailable { reason }
    }

    #[cfg(test)]
    pub(crate) fn resolved_executable(&self) -> Option<&ResolvedProviderExecutable> {
        match self {
            Self::Available {
                resolved_executable,
            } => Some(resolved_executable),
            Self::Unavailable { .. } => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable_reason(&self) -> Option<ProviderUnavailableReason> {
        match self {
            Self::Available { .. } => None,
            Self::Unavailable { reason } => Some(*reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRegistryEntry {
    provider: ProviderKind,
    display_name: &'static str,
    default_executable: ProviderExecutable,
    configured_executable: Option<ProviderExecutable>,
    availability: ProviderAvailability,
}

impl ProviderRegistryEntry {
    pub(crate) fn detect(
        provider: ProviderKind,
        configured_executable: Option<ProviderExecutable>,
        probe: impl FnOnce(&ProviderExecutable) -> ProviderAvailability,
    ) -> Self {
        let (display_name, default_executable) = provider_definition(provider);
        let default_executable = ProviderExecutable(default_executable.to_string());
        let availability = probe(
            configured_executable
                .as_ref()
                .unwrap_or(&default_executable),
        );
        Self {
            provider,
            display_name,
            default_executable,
            configured_executable,
            availability,
        }
    }

    pub(crate) fn provider(&self) -> ProviderKind {
        self.provider
    }

    pub(crate) fn display_name(&self) -> &'static str {
        self.display_name
    }

    pub(crate) fn default_executable(&self) -> &ProviderExecutable {
        &self.default_executable
    }

    pub(crate) fn configured_executable(&self) -> Option<&ProviderExecutable> {
        self.configured_executable.as_ref()
    }

    pub(crate) fn effective_executable(&self) -> &ProviderExecutable {
        self.configured_executable
            .as_ref()
            .unwrap_or(&self.default_executable)
    }

    pub(crate) fn is_available(&self) -> bool {
        matches!(self.availability, ProviderAvailability::Available { .. })
    }

    pub(crate) fn resolved_executable(&self) -> Option<&ResolvedProviderExecutable> {
        match &self.availability {
            ProviderAvailability::Available {
                resolved_executable,
            } => Some(resolved_executable),
            ProviderAvailability::Unavailable { .. } => None,
        }
    }

    pub(crate) fn unavailable_reason(&self) -> Option<ProviderUnavailableReason> {
        match self.availability {
            ProviderAvailability::Available { .. } => None,
            ProviderAvailability::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderRegistry {
    entries: Vec<ProviderRegistryEntry>,
}

impl ProviderRegistry {
    pub(crate) fn new(entries: Vec<ProviderRegistryEntry>) -> Result<Self, ProviderRegistryError> {
        let providers = entries
            .iter()
            .map(ProviderRegistryEntry::provider)
            .collect::<HashSet<_>>();
        if providers.len() != entries.len() {
            return Err(ProviderRegistryError::DuplicateProvider);
        }
        if ProviderKind::supported()
            .iter()
            .any(|provider| !providers.contains(provider))
            || providers.len() != ProviderKind::supported().len()
        {
            return Err(ProviderRegistryError::Incomplete);
        }
        Ok(Self { entries })
    }

    pub(crate) fn entries(&self) -> &[ProviderRegistryEntry] {
        &self.entries
    }

    pub(crate) fn entry(&self, provider: ProviderKind) -> &ProviderRegistryEntry {
        self.entries
            .iter()
            .find(|entry| entry.provider() == provider)
            .expect("ProviderRegistry contains every supported Provider")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderRegistryError {
    InvalidExecutable,
    InvalidResolvedExecutable,
    DuplicateProvider,
    Incomplete,
}

fn provider_definition(provider: ProviderKind) -> (&'static str, &'static str) {
    match provider {
        ProviderKind::Claude => ("Claude", "claude"),
        ProviderKind::Codex => ("Codex", "codex"),
    }
}
