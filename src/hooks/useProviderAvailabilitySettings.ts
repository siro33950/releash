import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useState } from "react";

export interface ProviderAvailabilityItem {
	provider: string;
	displayName: string;
	defaultExecutable: string;
	configuredExecutable: string | null;
	effectiveExecutable: string;
	available: boolean;
	resolvedExecutable: string | null;
	unavailableReason: string | null;
}

interface ProviderAvailabilitySnapshot {
	providers: ProviderAvailabilityItem[];
}

function draftsFrom(
	snapshot: ProviderAvailabilitySnapshot,
): Record<string, string> {
	return Object.fromEntries(
		snapshot.providers.map((provider) => [
			provider.provider,
			provider.configuredExecutable ?? "",
		]),
	);
}

function draftsAfterReset(
	provider: string,
	current: Record<string, string>,
	previous: ProviderAvailabilitySnapshot,
	next: ProviderAvailabilitySnapshot,
): Record<string, string> {
	const merged = draftsFrom(next);
	for (const entry of previous.providers) {
		if (entry.provider === provider) continue;
		const draft = current[entry.provider] ?? "";
		if (draft !== (entry.configuredExecutable ?? "")) {
			merged[entry.provider] = draft;
		}
	}
	return merged;
}

export function useProviderAvailabilitySettings(open: boolean) {
	const [snapshot, setSnapshot] = useState<ProviderAvailabilitySnapshot | null>(
		null,
	);
	const [drafts, setDrafts] = useState<Record<string, string>>({});
	const [loading, setLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const acceptSnapshot = useCallback((next: ProviderAvailabilitySnapshot) => {
		setSnapshot(next);
		setDrafts(draftsFrom(next));
	}, []);

	useEffect(() => {
		if (!open) return;
		let cancelled = false;
		setLoading(true);
		setError(null);
		setSnapshot(null);
		setDrafts({});
		invoke<ProviderAvailabilitySnapshot>("get_provider_availability")
			.then((next) => {
				if (!cancelled) acceptSnapshot(next);
			})
			.catch((cause) => {
				if (!cancelled) setError(String(cause));
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [open, acceptSnapshot]);

	const isDirty = useMemo(
		() =>
			snapshot?.providers.some(
				(provider) =>
					drafts[provider.provider] !== (provider.configuredExecutable ?? ""),
			) ?? false,
		[snapshot, drafts],
	);

	const setExecutable = useCallback((provider: string, executable: string) => {
		setDrafts((current) => ({ ...current, [provider]: executable }));
	}, []);

	const save = useCallback(async () => {
		if (!snapshot) return;
		setSaving(true);
		setError(null);
		try {
			let latest = snapshot;
			for (const provider of snapshot.providers) {
				const executable = drafts[provider.provider] ?? "";
				if (executable === (provider.configuredExecutable ?? "")) continue;
				latest = await invoke<ProviderAvailabilitySnapshot>(
					"update_provider_executable",
					{ provider: provider.provider, executable },
				);
				setSnapshot(latest);
			}
			acceptSnapshot(latest);
		} catch (cause) {
			setError(String(cause));
			throw cause;
		} finally {
			setSaving(false);
		}
	}, [snapshot, drafts, acceptSnapshot]);

	const reset = useCallback(
		async (provider: string) => {
			if (!snapshot) return;
			setSaving(true);
			setError(null);
			try {
				const next = await invoke<ProviderAvailabilitySnapshot>(
					"reset_provider_executable",
					{ provider },
				);
				setSnapshot(next);
				setDrafts(draftsAfterReset(provider, drafts, snapshot, next));
			} catch (cause) {
				setError(String(cause));
			} finally {
				setSaving(false);
			}
		},
		[snapshot, drafts],
	);

	const refresh = useCallback(async () => {
		setRefreshing(true);
		setError(null);
		try {
			acceptSnapshot(
				await invoke<ProviderAvailabilitySnapshot>(
					"refresh_provider_availability",
				),
			);
		} catch (cause) {
			setError(String(cause));
		} finally {
			setRefreshing(false);
		}
	}, [acceptSnapshot]);

	return {
		providers: snapshot?.providers ?? [],
		drafts,
		loading,
		saving,
		refreshing,
		error,
		isDirty,
		setExecutable,
		save,
		reset,
		refresh,
	};
}
