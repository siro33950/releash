import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useClientRefresh } from "./useClientRefresh";

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
	const clientRefresh = useClientRefresh(open);
	const wasOpen = useRef(false);
	const [snapshot, setSnapshot] = useState<ProviderAvailabilitySnapshot | null>(
		null,
	);
	const [drafts, setDrafts] = useState<Record<string, string>>({});
	const form = useRef({ snapshot, drafts });
	form.current = { snapshot, drafts };
	const [loading, setLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [resetting, setResetting] = useState<Record<string, boolean>>({});
	const [error, setError] = useState<string | null>(null);

	const acceptSnapshot = useCallback((next: ProviderAvailabilitySnapshot) => {
		setSnapshot(next);
		setDrafts(draftsFrom(next));
	}, []);

	useEffect(() => {
		const preserveDraft = wasOpen.current;
		wasOpen.current = open;
		if (!open) return;
		if (!preserveDraft) {
			setSnapshot(null);
			setDrafts({});
		}
		let cancelled = false;
		setLoading(true);
		setError(null);
		invoke("get_provider_availability")
			.then((next) => {
				if (cancelled || clientRefresh.aborted) return;
				const current = form.current;
				setSnapshot(next);
				setDrafts(
					preserveDraft && current.snapshot
						? draftsAfterReset("", current.drafts, current.snapshot, next)
						: draftsFrom(next),
				);
			})
			.catch((cause) => {
				if (!cancelled && !clientRefresh.aborted)
					setError(getErrorMessage(cause));
			})
			.finally(() => {
				if (!cancelled && !clientRefresh.aborted) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [open, clientRefresh]);

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
				latest = await invoke("update_provider_executable", {
					provider: provider.provider,
					executable,
				});
				setSnapshot(latest);
			}
			acceptSnapshot(latest);
		} catch (cause) {
			setError(getErrorMessage(cause));
			throw cause;
		} finally {
			setSaving(false);
		}
	}, [snapshot, drafts, acceptSnapshot]);

	const reset = useCallback(
		async (provider: string) => {
			if (!snapshot) return;
			setResetting((current) => ({ ...current, [provider]: true }));
			setError(null);
			try {
				const next = await invoke("reset_provider_executable", { provider });
				setSnapshot(next);
				const current = form.current;
				setDrafts(
					draftsAfterReset(
						provider,
						current.drafts,
						current.snapshot ?? snapshot,
						next,
					),
				);
			} catch (cause) {
				setError(getErrorMessage(cause));
			} finally {
				setResetting((current) => ({ ...current, [provider]: false }));
			}
		},
		[snapshot],
	);

	const refresh = useCallback(async () => {
		setRefreshing(true);
		setError(null);
		try {
			acceptSnapshot(await invoke("refresh_provider_availability"));
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setRefreshing(false);
		}
	}, [acceptSnapshot]);

	return {
		providers: snapshot?.providers ?? [],
		drafts,
		loading,
		saving: saving || Object.values(resetting).some(Boolean),
		refreshing,
		error,
		isDirty,
		setExecutable,
		save,
		reset,
		refresh,
	};
}
