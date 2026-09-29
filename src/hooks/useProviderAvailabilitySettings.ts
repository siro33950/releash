import { useCallback, useEffect, useMemo, useReducer, useState } from "react";
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
	receivedConfigured?: string | null;
}

interface FormState {
	snapshot: ProviderAvailabilitySnapshot | null;
	drafts: Record<string, string>;
	pending: Record<string, PendingDraft>;
}

type FormAction =
	| { type: "receive"; snapshot: ProviderAvailabilitySnapshot }
	| { type: "start"; provider: string; operation: PendingDraft }
	| { type: "complete" | "fail"; provider: string }
	| { type: "input"; provider: string; executable: string }
	| { type: "clear" };

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
	pending: Readonly<Record<string, PendingDraft>>,
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
			if (
				nextEntry.configuredExecutable !== operation.configured ||
				operation.completed
			) {
				if (draft !== operation.draft) merged[entry.provider] = draft;
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

function formReducer(state: FormState, action: FormAction): FormState {
	switch (action.type) {
		case "clear":
			return { snapshot: null, drafts: {}, pending: {} };
		case "input":
			return {
				...state,
				drafts: { ...state.drafts, [action.provider]: action.executable },
			};
		case "start":
			return {
				...state,
				pending: { ...state.pending, [action.provider]: action.operation },
			};
		case "fail": {
			const { [action.provider]: _, ...pending } = state.pending;
			return { ...state, pending };
		}
		case "complete": {
			const operation = state.pending[action.provider];
			if (!operation) return state;
			if (operation.receivedConfigured === undefined) {
				return {
					...state,
					pending: {
						...state.pending,
						[action.provider]: { ...operation, completed: true },
					},
				};
			}
			const { [action.provider]: _, ...pending } = state.pending;
			return {
				...state,
				pending,
				drafts:
					state.drafts[action.provider] === operation.draft
						? {
								...state.drafts,
								[action.provider]: operation.receivedConfigured ?? "",
							}
						: state.drafts,
			};
		}
		case "receive": {
			const drafts = state.snapshot
				? mergeDrafts(
						state.drafts,
						state.snapshot,
						action.snapshot,
						state.pending,
					)
				: draftsFrom(action.snapshot);
			const pending = Object.fromEntries(
				Object.entries(state.pending).flatMap(([provider, operation]) => {
					const entry = action.snapshot.providers.find(
						(item) => item.provider === provider,
					);
					if (
						!entry ||
						entry.configuredExecutable !== operation.configured ||
						operation.completed
					)
						return [];
					return [
						[
							provider,
							{ ...operation, receivedConfigured: entry.configuredExecutable },
						],
					];
				}),
			);
			return { snapshot: action.snapshot, drafts, pending };
		}
	}
}

export function useProviderAvailabilitySettings(open: boolean) {
	const subscription = useStateSubscriptionResult(
		open ? "provider-availability" : null,
	);
	const [form, dispatch] = useReducer(formReducer, {
		snapshot: null,
		drafts: {},
		pending: {},
	});
	const { snapshot, drafts } = form;
	const [saving, setSaving] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [resetting, setResetting] = useState<Record<string, boolean>>({});
	const [error, setError] = useState<string | null>(null);
	useEffect(() => {
		if (!open) {
			dispatch({ type: "clear" });
			setError(null);
		}
	}, [open]);

	const received = subscription.value;
	useEffect(() => {
		if (received) dispatch({ type: "receive", snapshot: received });
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
		dispatch({ type: "input", provider, executable });
	}, []);

	const save = useCallback(async () => {
		if (!snapshot) return;
		setSaving(true);
		setError(null);
		try {
			for (const provider of snapshot.providers) {
				const executable = drafts[provider.provider] ?? "";
				if (executable === (provider.configuredExecutable ?? "")) continue;
				dispatch({
					type: "start",
					provider: provider.provider,
					operation: {
						draft: executable,
						configured: provider.configuredExecutable,
						completed: false,
					},
				});
				try {
					await invoke("update_provider_executable", {
						provider: provider.provider,
						executable,
					});
					dispatch({ type: "complete", provider: provider.provider });
				} catch (cause) {
					dispatch({ type: "fail", provider: provider.provider });
					throw cause;
				}
			}
		} catch (cause) {
			setError(getErrorMessage(cause));
			throw cause;
		} finally {
			setSaving(false);
		}
	}, [snapshot, drafts]);

	const reset = useCallback(
		async (provider: string) => {
			if (!snapshot) return;
			setResetting((current) => ({ ...current, [provider]: true }));
			setError(null);
			dispatch({
				type: "start",
				provider,
				operation: {
					draft: drafts[provider] ?? "",
					configured:
						snapshot.providers.find((item) => item.provider === provider)
							?.configuredExecutable ?? null,
					completed: false,
				},
			});
			try {
				await invoke("reset_provider_executable", { provider });
				dispatch({ type: "complete", provider });
			} catch (cause) {
				dispatch({ type: "fail", provider });
				setError(getErrorMessage(cause));
			} finally {
				setResetting((current) => ({ ...current, [provider]: false }));
			}
		},
		[snapshot, drafts],
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
