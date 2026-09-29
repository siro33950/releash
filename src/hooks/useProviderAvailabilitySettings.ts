import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useStateSubscriptionResult } from "./useStateSubscription";

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

interface PendingDraft {
	draft: string;
	configured: string | null;
	completed: boolean;
	received: boolean;
	receivedConfigured: string | null;
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

function mergeDrafts(
	current: Record<string, string>,
	previous: ProviderAvailabilitySnapshot,
	next: ProviderAvailabilitySnapshot,
	pending: Record<string, PendingDraft>,
): Record<string, string> {
	const merged = draftsFrom(next);
	for (const entry of previous.providers) {
		const draft = current[entry.provider] ?? "";
		const operation = pending[entry.provider];
		if (operation) {
			const nextEntry = next.providers.find(
				(item) => item.provider === entry.provider,
			);
			if (!nextEntry) continue;
			operation.received = true;
			operation.receivedConfigured = nextEntry.configuredExecutable;
			if (
				nextEntry.configuredExecutable !== operation.configured ||
				operation.completed
			) {
				if (draft !== operation.draft) merged[entry.provider] = draft;
				delete pending[entry.provider];
			} else {
				merged[entry.provider] = draft;
			}
			continue;
		}
		if (draft !== (entry.configuredExecutable ?? "")) {
			merged[entry.provider] = draft;
		}
	}
	return merged;
}

export function useProviderAvailabilitySettings(open: boolean) {
	const subscription = useStateSubscriptionResult(
		open ? "provider-availability" : null,
	);
	const [snapshot, setSnapshot] = useState<ProviderAvailabilitySnapshot | null>(
		null,
	);
	const [drafts, setDrafts] = useState<Record<string, string>>({});
	const form = useRef({ snapshot, drafts });
	const pendingDrafts = useRef<Record<string, PendingDraft>>({});
	form.current = { snapshot, drafts };
	const [saving, setSaving] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [resetting, setResetting] = useState<Record<string, boolean>>({});
	const [error, setError] = useState<string | null>(null);
	const completePending = useCallback((provider: string) => {
		const pending = pendingDrafts.current[provider];
		if (!pending) return;
		pending.completed = true;
		if (!pending.received) return;
		setDrafts((current) =>
			current[provider] === pending.draft
				? { ...current, [provider]: pending.receivedConfigured ?? "" }
				: current,
		);
		delete pendingDrafts.current[provider];
	}, []);

	useEffect(() => {
		if (open) return;
		setSnapshot(null);
		setDrafts({});
		pendingDrafts.current = {};
		setError(null);
	}, [open]);

	const received = subscription.value;
	useEffect(() => {
		if (!received) return;
		const current = form.current;
		setSnapshot(received);
		setDrafts(
			current.snapshot
				? mergeDrafts(
						current.drafts,
						current.snapshot,
						received,
						pendingDrafts.current,
					)
				: draftsFrom(received),
		);
	}, [received]);

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
			for (const provider of snapshot.providers) {
				const executable = drafts[provider.provider] ?? "";
				if (executable === (provider.configuredExecutable ?? "")) continue;
				pendingDrafts.current[provider.provider] = {
					draft: executable,
					configured: provider.configuredExecutable,
					completed: false,
					received: false,
					receivedConfigured: null,
				};
				try {
					await invoke("update_provider_executable", {
						provider: provider.provider,
						executable,
					});
					completePending(provider.provider);
				} catch (cause) {
					delete pendingDrafts.current[provider.provider];
					throw cause;
				}
			}
		} catch (cause) {
			setError(getErrorMessage(cause));
			throw cause;
		} finally {
			setSaving(false);
		}
	}, [snapshot, drafts, completePending]);

	const reset = useCallback(
		async (provider: string) => {
			if (!snapshot) return;
			setResetting((current) => ({ ...current, [provider]: true }));
			setError(null);
			pendingDrafts.current[provider] = {
				draft: drafts[provider] ?? "",
				configured:
					snapshot.providers.find((item) => item.provider === provider)
						?.configuredExecutable ?? null,
				completed: false,
				received: false,
				receivedConfigured: null,
			};
			try {
				await invoke("reset_provider_executable", { provider });
				completePending(provider);
			} catch (cause) {
				delete pendingDrafts.current[provider];
				setError(getErrorMessage(cause));
			} finally {
				setResetting((current) => ({ ...current, [provider]: false }));
			}
		},
		[snapshot, drafts, completePending],
	);

	const refresh = useCallback(async () => {
		setRefreshing(true);
		setError(null);
		try {
			await invoke("refresh_provider_availability");
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setRefreshing(false);
		}
	}, []);

	return {
		providers: snapshot?.providers ?? [],
		drafts,
		loading: open && !snapshot && !subscription.error,
		saving: saving || Object.values(resetting).some(Boolean),
		refreshing,
		error: error ?? subscription.error,
		isDirty,
		setExecutable,
		save,
		reset,
		refresh,
	};
}
