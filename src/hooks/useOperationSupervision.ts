import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	getAcceptedSendOperation,
	redispatchPendingLifecycleAttempts,
	redispatchPendingPermissionResponseAttempts,
	redispatchPendingSendAttempts,
	redispatchPendingStopAttempts,
	type SendOperationView,
} from "@/hooks/useSessionStore";

export interface RecoveryCapability {
	obligation_id: string;
	category:
		| "turn_execution"
		| "queue_execution"
		| "permission_delivery"
		| "provider_establish"
		| "terminal_commit"
		| "backend_recovery"
		| "session_close"
		| "workflow_shutdown"
		| "recovery_publication"
		| "unknown";
	original_identity: string;
	owner: string;
	partition:
		| "owner"
		| "closed_session"
		| "archived_session"
		| "unowned_runtime";
	shutdown_plan: { plan_id: string; epoch: string } | null;
	revision: string;
	state: "pending" | "failed";
	known_status:
		| "prepared"
		| "pending"
		| "effect_reserved"
		| "running"
		| "waiting_approval"
		| "reconciliation_required"
		| "failed"
		| "unknown";
	safe_label: string;
	actions: string[];
	action_identities: Array<{
		action_id: string;
		action: string;
		origin_revision: string;
	}>;
}

interface PendingRecoveryPage {
	entries: RecoveryCapability[];
	next_cursor: string | null;
}

interface PendingCallerAttemptPage {
	entries: PendingCallerAttempt[];
	next_cursor: string | null;
}

const MAX_ATTEMPT_PAGES_PER_REFRESH = 16;
const MAX_ATTEMPTS_PER_REFRESH = 512;

async function loadAttemptPages(
	scopeId: string,
	initialCursor: string | null,
): Promise<{ entries: PendingCallerAttempt[]; nextCursor: string | null }> {
	const entries: PendingCallerAttempt[] = [];
	let cursor = initialCursor;
	for (
		let pageIndex = 0;
		pageIndex < MAX_ATTEMPT_PAGES_PER_REFRESH;
		pageIndex += 1
	) {
		const page = await invoke<PendingCallerAttemptPage>(
			"list_pending_agent_attempts",
			{ scopeId, limit: 32, cursor },
		);
		entries.push(...page.entries);
		cursor = page.next_cursor;
		if (cursor === null || entries.length >= MAX_ATTEMPTS_PER_REFRESH) break;
	}
	return {
		entries: entries.slice(0, MAX_ATTEMPTS_PER_REFRESH),
		nextCursor: cursor,
	};
}

export interface PendingCallerAttempt {
	kind:
		| "send"
		| "permission_response"
		| "stop"
		| "session_lifecycle"
		| "application_quit";
	caller_request_id: string;
	operation_id: string | null;
	resolution: "pending" | "accepted" | "rejected_before_commit";
}

interface MigrationProjection {
	migration_id: string;
	phase: string;
	next_source_ordinal: string;
	total_source_count: string;
	safe_failure: string | null;
	correlation_id: string | null;
}

interface ShutdownProjection {
	plan_id: string;
	epoch: string;
	phase: string;
	outcome: string | null;
	actions: string[];
}

interface ShutdownOutcomeUnknown {
	operation_id: string;
	intent: { type: "exit" | "restart"; code: number };
}

type CurrentShutdownResult =
	| { type: "current"; plan: ShutdownProjection | null }
	| ({ type: "outcome_unknown" } & ShutdownOutcomeUnknown);

export interface ShutdownTargetCapability {
	ordinal: string;
	target_key: string;
	target_id: string;
	kind: string;
	effect_identity: string;
	state: string;
	observation:
		| {
				type: "provider_observation";
				observation_ref: string;
				proof_sha256: string;
		  }
		| { type: "confirmed_no_effect"; proof_sha256: string }
		| {
				type: "exit_coupled_outcome_unknown";
				plan_id: string;
				epoch: string;
		  }
		| null;
	revision: string;
	actions: string[];
	action_identities: Array<{
		action_id: string;
		action: string;
		origin_revision: string;
	}>;
}

interface ShutdownPlanPage {
	plan: ShutdownProjection;
	targets: ShutdownTargetCapability[];
	next_cursor: string | null;
}

export interface OperationSupervisionState {
	sendOperation: SendOperationView | null;
	attempts: PendingCallerAttempt[];
	operationReadbacks: Array<{
		kind: PendingCallerAttempt["kind"];
		operation_id: string;
		result: unknown;
	}>;
	recovery: RecoveryCapability[];
	migration: MigrationProjection | null;
	shutdown: ShutdownProjection | null;
	shutdownOutcomeUnknown: ShutdownOutcomeUnknown | null;
	shutdownTargets: ShutdownTargetCapability[];
	refreshing: boolean;
	error: string | null;
}

const EMPTY: OperationSupervisionState = {
	sendOperation: null,
	attempts: [],
	operationReadbacks: [],
	recovery: [],
	migration: null,
	shutdown: null,
	shutdownOutcomeUnknown: null,
	shutdownTargets: [],
	refreshing: false,
	error: null,
};

const QUIT_ATTEMPT_STORAGE_KEY = "releash:application-quit-attempt:v1";
const ADOPTED_OPERATION_STORAGE_KEY = "releash:adopted-operation-identities:v1";

interface AdoptedOperationIdentity {
	kind: PendingCallerAttempt["kind"];
	operation_id: string;
	scope_id: string;
}

function operationReadbackCommand(kind: PendingCallerAttempt["kind"]): string {
	switch (kind) {
		case "send":
			return "get_agent_send_operation";
		case "permission_response":
			return "get_agent_permission_response_operation";
		case "stop":
			return "get_stop_operation";
		case "session_lifecycle":
			return "get_session_lifecycle_operation";
		case "application_quit":
			return "get_application_quit_operation";
	}
}

function loadAdoptedOperationIdentities(): AdoptedOperationIdentity[] {
	try {
		const value = JSON.parse(
			globalThis.localStorage.getItem(ADOPTED_OPERATION_STORAGE_KEY) ?? "[]",
		) as AdoptedOperationIdentity[];
		return Array.isArray(value) ? value.slice(-512) : [];
	} catch {
		return [];
	}
}

function rememberAdoptedOperation(identity: AdoptedOperationIdentity): void {
	const current = loadAdoptedOperationIdentities().filter(
		(entry) =>
			entry.kind !== identity.kind ||
			entry.operation_id !== identity.operation_id,
	);
	globalThis.localStorage.setItem(
		ADOPTED_OPERATION_STORAGE_KEY,
		JSON.stringify([...current, identity].slice(-512)),
	);
}

interface QuitAttemptSnapshot {
	requestId: string;
	intent: { type: "exit" | "restart"; code: number };
}

function loadQuitAttempt(): QuitAttemptSnapshot | null {
	const saved = globalThis.localStorage.getItem(QUIT_ATTEMPT_STORAGE_KEY);
	if (!saved) return null;
	try {
		const parsed = JSON.parse(saved) as Partial<QuitAttemptSnapshot>;
		if (
			typeof parsed.requestId === "string" &&
			(parsed.intent?.type === "exit" || parsed.intent?.type === "restart") &&
			typeof parsed.intent.code === "number" &&
			Number.isInteger(parsed.intent.code)
		) {
			return parsed as QuitAttemptSnapshot;
		}
		return null;
	} catch {
		// The previous format retained only the id; RetryQuit has always used
		// the exact exit/0 payload, so this is a lossless one-time upgrade.
		return { requestId: saved, intent: { type: "exit", code: 0 } };
	}
}

async function redispatchPendingQuitAttempts(
	requestIds: ReadonlySet<string>,
): Promise<void> {
	const snapshot = loadQuitAttempt();
	if (!snapshot || !requestIds.has(snapshot.requestId)) return;
	const outcome = await invoke<{ type: string }>("request_application_quit", {
		request: {
			request_id: snapshot.requestId,
			intent: snapshot.intent,
		},
	});
	if (outcome.type !== "outcome_unknown") {
		globalThis.localStorage.removeItem(QUIT_ATTEMPT_STORAGE_KEY);
	}
}

/**
 * A bounded UI mirror of backend-owned supervision projections.  It never
 * derives capabilities or retries an effect by itself.
 */
export function useOperationSupervision(sessionId: string) {
	const [state, setState] = useState<OperationSupervisionState>(EMPTY);
	const sessionAttemptCursor = useRef<string | null>(null);
	const applicationAttemptCursor = useRef<string | null>(null);
	const refresh = useCallback(async () => {
		setState((current) => ({ ...current, refreshing: true }));
		try {
			const [
				localSendOperation,
				sessionAttempts,
				applicationAttempts,
				recovery,
				migrationResult,
				shutdownResult,
			] = await Promise.all([
				getAcceptedSendOperation(sessionId),
				loadAttemptPages(sessionId, sessionAttemptCursor.current),
				loadAttemptPages("application", applicationAttemptCursor.current),
				invoke<PendingRecoveryPage>("list_pending_agent_recovery", {
					limit: 32,
					partition: null,
					owner: sessionId,
					shutdownPlanId: null,
					shutdownEpoch: null,
					cursor: null,
				}),
				invoke<{
					type: "current";
					migration: MigrationProjection | null;
				}>("get_local_store_migration"),
				invoke<CurrentShutdownResult>("get_application_shutdown"),
			]);
			const attempts = [
				...sessionAttempts.entries,
				...applicationAttempts.entries,
			];
			sessionAttemptCursor.current = sessionAttempts.nextCursor;
			applicationAttemptCursor.current = applicationAttempts.nextCursor;
			await Promise.all([
				redispatchPendingSendAttempts(
					new Set(
						attempts
							.filter(
								(attempt) =>
									attempt.kind === "send" && attempt.resolution === "pending",
							)
							.map((attempt) => attempt.caller_request_id),
					),
				),
				redispatchPendingPermissionResponseAttempts(
					new Set(
						attempts
							.filter(
								(attempt) =>
									attempt.kind === "permission_response" &&
									attempt.resolution === "pending",
							)
							.map((attempt) => attempt.caller_request_id),
					),
				),
				redispatchPendingStopAttempts(
					new Set(
						attempts
							.filter(
								(attempt) =>
									attempt.kind === "stop" && attempt.resolution === "pending",
							)
							.map((attempt) => attempt.caller_request_id),
					),
				),
				redispatchPendingLifecycleAttempts(
					new Set(
						attempts
							.filter(
								(attempt) =>
									attempt.kind === "session_lifecycle" &&
									attempt.resolution === "pending",
							)
							.map((attempt) => attempt.caller_request_id),
					),
				),
				redispatchPendingQuitAttempts(
					new Set(
						attempts
							.filter(
								(attempt) =>
									attempt.kind === "application_quit" &&
									attempt.resolution === "pending",
							)
							.map((attempt) => attempt.caller_request_id),
					),
				),
			]);
			const acceptedAttempts = attempts.filter(
				(attempt) => attempt.resolution === "accepted",
			);
			const seenOperationIdentities = new Set<string>();
			const adoptedReadbacks = new Map<
				string,
				OperationSupervisionState["operationReadbacks"][number]
			>();
			const operationReadbacks: OperationSupervisionState["operationReadbacks"] =
				[];
			for (const attempt of acceptedAttempts) {
				const operationId = attempt.operation_id ?? attempt.caller_request_id;
				const identity = `${attempt.kind}:${operationId}`;
				let readback = adoptedReadbacks.get(identity);
				if (!readback) {
					const command = operationReadbackCommand(attempt.kind);
					const adoptedReadback: OperationSupervisionState["operationReadbacks"][number] =
						{
							kind: attempt.kind,
							operation_id: operationId,
							result: await invoke(command, { operationId }),
						};
					readback = adoptedReadback;
					adoptedReadbacks.set(identity, adoptedReadback);
					seenOperationIdentities.add(identity);
					operationReadbacks.push(adoptedReadback);
					setState((current) => ({
						...current,
						operationReadbacks: [
							...current.operationReadbacks,
							adoptedReadback,
						],
					}));
				}
				// Adoption is attempt-local. A crash before this point leaves the
				// caller attempt unacknowledged; a crash after it can recover by the
				// durable operation identity retained in renderer state.
				rememberAdoptedOperation({
					kind: attempt.kind,
					operation_id: operationId,
					scope_id:
						attempt.kind === "application_quit" ? "application" : sessionId,
				});
				await invoke("acknowledge_agent_attempt", {
					kind: attempt.kind,
					callerRequestId: attempt.caller_request_id,
				}).catch(() => undefined);
			}
			for (const adopted of loadAdoptedOperationIdentities()) {
				if (
					adopted.scope_id !== sessionId &&
					adopted.scope_id !== "application"
				) {
					continue;
				}
				const identity = `${adopted.kind}:${adopted.operation_id}`;
				if (seenOperationIdentities.has(identity)) continue;
				seenOperationIdentities.add(identity);
				const command = operationReadbackCommand(adopted.kind);
				try {
					operationReadbacks.push({
						kind: adopted.kind,
						operation_id: adopted.operation_id,
						result: await invoke(command, {
							operationId: adopted.operation_id,
						}),
					});
				} catch {
					// A terminal operation may eventually age out of backend retention.
					// It must not prevent newer supervision entries from being adopted.
				}
			}
			const backendSendOperations = operationReadbacks
				.filter((entry) => entry.kind === "send")
				.map((entry) => entry.result as SendOperationView);
			const latestBackendSendOperation =
				backendSendOperations[backendSendOperations.length - 1];
			const sendOperation =
				localSendOperation ?? latestBackendSendOperation ?? null;
			const shutdownTargets: ShutdownTargetCapability[] = [];
			const shutdown =
				shutdownResult.type === "current" ? shutdownResult.plan : null;
			const shutdownOutcomeUnknown =
				shutdownResult.type === "outcome_unknown"
					? {
							operation_id: shutdownResult.operation_id,
							intent: shutdownResult.intent,
						}
					: null;
			if (shutdown) {
				const page = await invoke<ShutdownPlanPage>("get_shutdown_plan", {
					planId: shutdown.plan_id,
					epoch: shutdown.epoch,
					limit: 128,
					cursor: null,
				});
				shutdownTargets.push(...page.targets);
			}
			setState({
				sendOperation,
				attempts,
				operationReadbacks,
				recovery: recovery.entries,
				migration: migrationResult.migration,
				shutdown,
				shutdownOutcomeUnknown,
				shutdownTargets,
				refreshing: false,
				error: null,
			});
		} catch (error) {
			if (String(error).includes("cursor")) {
				sessionAttemptCursor.current = null;
				applicationAttemptCursor.current = null;
			}
			setState((current) => ({
				...current,
				refreshing: false,
				error: String(error),
			}));
		}
	}, [sessionId]);

	useEffect(() => {
		sessionAttemptCursor.current = null;
		applicationAttemptCursor.current = null;
		void refresh();
		const interval = globalThis.setInterval(() => void refresh(), 2_000);
		return () => globalThis.clearInterval(interval);
	}, [refresh]);

	const requestRecovery = useCallback(
		async (entry: RecoveryCapability, action: string) => {
			const issued = entry.action_identities.find(
				(identity) => identity.action === action,
			);
			if (!entry.actions.includes(action) || !issued) {
				throw new Error("The backend did not grant that recovery capability");
			}
			await invoke("resolve_pending_recovery_action", {
				request: {
					action_id: issued.action_id,
					obligation_id: entry.obligation_id,
					origin_revision: entry.revision,
					action,
				},
			});
			await refresh();
		},
		[refresh],
	);

	const retryShutdownTarget = useCallback(
		async (target: ShutdownTargetCapability) => {
			const issued = target.action_identities.find(
				(identity) => identity.action === "retry_same_effect",
			);
			if (
				!state.shutdown ||
				!target.actions.includes("retry_same_effect") ||
				!issued
			) {
				throw new Error(
					"The backend did not expose a retryable shutdown target",
				);
			}
			await invoke("resolve_shutdown_target_action", {
				request: {
					action_id: issued.action_id,
					plan_id: state.shutdown.plan_id,
					epoch: state.shutdown.epoch,
					ordinal: target.ordinal,
					target_key: target.target_key,
					origin_revision: issued.origin_revision,
					action: "retry_same_effect",
				},
			});
			await refresh();
		},
		[refresh, state.shutdown],
	);

	const retryQuit = useCallback(async () => {
		if (!state.shutdown?.actions.includes("retry_quit")) {
			throw new Error("The backend did not expose RetryQuit");
		}
		const snapshot = loadQuitAttempt() ?? {
			requestId: `quit-${crypto.randomUUID()}`,
			intent: { type: "exit" as const, code: 0 },
		};
		globalThis.localStorage.setItem(
			QUIT_ATTEMPT_STORAGE_KEY,
			JSON.stringify(snapshot),
		);
		const outcome = await invoke<{ type: string }>("request_application_quit", {
			request: {
				request_id: snapshot.requestId,
				intent: snapshot.intent,
			},
		});
		if (outcome.type !== "outcome_unknown") {
			globalThis.localStorage.removeItem(QUIT_ATTEMPT_STORAGE_KEY);
		}
		await refresh();
	}, [refresh, state.shutdown]);

	return { state, refresh, requestRecovery, retryShutdownTarget, retryQuit };
}
