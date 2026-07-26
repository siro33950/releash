import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	getAcceptedSendOperation,
	listAcceptedPermissionResponseOperations,
	type PermissionResponseOperationView,
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
	shutdown_plan: { shutdown_id: string } | null;
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

interface ShutdownProjection {
	shutdown_id: string;
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
				shutdown_id: string;
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

export interface OperationReadback {
	kind: PendingCallerAttempt["kind"];
	operation_id: string;
	result: unknown;
}

/** Session scope (S4 / S5 / S6): the operations owned by one Session. */
export interface OperationSupervisionState {
	sendOperation: SendOperationView | null;
	permissionResponseOperations: PermissionResponseOperationView[];
	attempts: PendingCallerAttempt[];
	operationReadbacks: OperationReadback[];
	recovery: RecoveryCapability[];
	error: string | null;
}

/** Application scope (S10): the single quit flight and its targets. */
export interface ApplicationShutdownSupervisionState {
	attempts: PendingCallerAttempt[];
	operationReadbacks: OperationReadback[];
	shutdown: ShutdownProjection | null;
	shutdownOutcomeUnknown: ShutdownOutcomeUnknown | null;
	shutdownTargets: ShutdownTargetCapability[];
	error: string | null;
}

const EMPTY: OperationSupervisionState = {
	sendOperation: null,
	permissionResponseOperations: [],
	attempts: [],
	operationReadbacks: [],
	recovery: [],
	error: null,
};

const EMPTY_SHUTDOWN: ApplicationShutdownSupervisionState = {
	attempts: [],
	operationReadbacks: [],
	shutdown: null,
	shutdownOutcomeUnknown: null,
	shutdownTargets: [],
	error: null,
};

/**
 * The supervision poll runs on a fixed interval. Emitting a fresh state object
 * for an unchanged backend snapshot would re-render every consumer twice per
 * cycle, so keep the previous object whenever the mirrored projection is equal.
 */
function nextSupervisionState<T>(current: T, next: T): T {
	return JSON.stringify(current) === JSON.stringify(next) ? current : next;
}

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
 * Adopt every accepted caller attempt of one scope by its durable operation
 * identity, then replay the identities this renderer already adopted. Both
 * halves are scope-local so Session and application supervision never publish
 * each other's operations.
 */
async function adoptAcceptedAttempts(
	scopeId: string,
	attempts: PendingCallerAttempt[],
	onAdopted: (readback: OperationReadback) => void,
): Promise<OperationReadback[]> {
	const seenOperationIdentities = new Set<string>();
	const adoptedReadbacks = new Map<string, OperationReadback>();
	const operationReadbacks: OperationReadback[] = [];
	for (const attempt of attempts.filter(
		(entry) => entry.resolution === "accepted",
	)) {
		const operationId = attempt.operation_id ?? attempt.caller_request_id;
		const identity = `${attempt.kind}:${operationId}`;
		if (!adoptedReadbacks.has(identity)) {
			const command = operationReadbackCommand(attempt.kind);
			const adoptedReadback: OperationReadback = {
				kind: attempt.kind,
				operation_id: operationId,
				result: await invoke(command, { operationId }),
			};
			adoptedReadbacks.set(identity, adoptedReadback);
			seenOperationIdentities.add(identity);
			operationReadbacks.push(adoptedReadback);
			onAdopted(adoptedReadback);
		}
		// Adoption is attempt-local. A crash before this point leaves the
		// caller attempt unacknowledged; a crash after it can recover by the
		// durable operation identity retained in renderer state.
		rememberAdoptedOperation({
			kind: attempt.kind,
			operation_id: operationId,
			scope_id: scopeId,
		});
		await invoke("acknowledge_agent_attempt", {
			kind: attempt.kind,
			callerRequestId: attempt.caller_request_id,
		}).catch(() => undefined);
	}
	for (const adopted of loadAdoptedOperationIdentities()) {
		if (adopted.scope_id !== scopeId) continue;
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
	return operationReadbacks;
}

/**
 * A bounded UI mirror of the backend-owned supervision projections owned by one
 * Session. It never derives capabilities or retries an effect by itself.
 */
export function useOperationSupervision(sessionId: string) {
	const [state, setState] = useState<OperationSupervisionState>(EMPTY);
	const sessionAttemptCursor = useRef<string | null>(null);
	const refresh = useCallback(async () => {
		try {
			const [
				localSendOperation,
				permissionResponseOperations,
				sessionAttempts,
				recovery,
			] = await Promise.all([
				getAcceptedSendOperation(sessionId),
				listAcceptedPermissionResponseOperations(sessionId),
				loadAttemptPages(sessionId, sessionAttemptCursor.current),
				invoke<PendingRecoveryPage>("list_pending_agent_recovery", {
					limit: 32,
					partition: null,
					owner: sessionId,
					shutdownId: null,
					cursor: null,
				}),
			]);
			const attempts = sessionAttempts.entries;
			sessionAttemptCursor.current = sessionAttempts.nextCursor;
			const pendingRequestIds = (kind: PendingCallerAttempt["kind"]) =>
				new Set(
					attempts
						.filter(
							(attempt) =>
								attempt.kind === kind && attempt.resolution === "pending",
						)
						.map((attempt) => attempt.caller_request_id),
				);
			await Promise.all([
				redispatchPendingSendAttempts(pendingRequestIds("send")),
				redispatchPendingPermissionResponseAttempts(
					pendingRequestIds("permission_response"),
				),
				redispatchPendingStopAttempts(pendingRequestIds("stop")),
				redispatchPendingLifecycleAttempts(
					pendingRequestIds("session_lifecycle"),
				),
			]);
			const operationReadbacks = await adoptAcceptedAttempts(
				sessionId,
				attempts,
				(adoptedReadback) => {
					setState((current) => ({
						...current,
						operationReadbacks: [
							...current.operationReadbacks,
							adoptedReadback,
						],
					}));
				},
			);
			const backendSendOperations = operationReadbacks
				.filter((entry) => entry.kind === "send")
				.map((entry) => entry.result as SendOperationView);
			const latestBackendSendOperation =
				backendSendOperations[backendSendOperations.length - 1];
			const sendOperation =
				localSendOperation ?? latestBackendSendOperation ?? null;
			setState((current) =>
				nextSupervisionState(current, {
					sendOperation,
					permissionResponseOperations,
					attempts,
					operationReadbacks,
					recovery: recovery.entries,
					error: null,
				}),
			);
		} catch (error) {
			if (String(error).includes("cursor")) {
				sessionAttemptCursor.current = null;
			}
			setState((current) =>
				nextSupervisionState(current, {
					...current,
					error: String(error),
				}),
			);
		}
	}, [sessionId]);

	useEffect(() => {
		sessionAttemptCursor.current = null;
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

	return { state, refresh, requestRecovery };
}

/**
 * A bounded UI mirror of the single application quit flight (S10). It is
 * application-scoped on purpose: a Session surface must not present another
 * scope's failure or action.
 */
export function useApplicationShutdownSupervision() {
	const [state, setState] =
		useState<ApplicationShutdownSupervisionState>(EMPTY_SHUTDOWN);
	const applicationAttemptCursor = useRef<string | null>(null);
	const refresh = useCallback(async () => {
		try {
			const [applicationAttempts, shutdownResult] = await Promise.all([
				loadAttemptPages("application", applicationAttemptCursor.current),
				invoke<CurrentShutdownResult>("get_application_shutdown"),
			]);
			const attempts = applicationAttempts.entries;
			applicationAttemptCursor.current = applicationAttempts.nextCursor;
			await redispatchPendingQuitAttempts(
				new Set(
					attempts
						.filter(
							(attempt) =>
								attempt.kind === "application_quit" &&
								attempt.resolution === "pending",
						)
						.map((attempt) => attempt.caller_request_id),
				),
			);
			const operationReadbacks = await adoptAcceptedAttempts(
				"application",
				attempts,
				(adoptedReadback) => {
					setState((current) => ({
						...current,
						operationReadbacks: [
							...current.operationReadbacks,
							adoptedReadback,
						],
					}));
				},
			);
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
					shutdownId: shutdown.shutdown_id,
					limit: 128,
					cursor: null,
				});
				shutdownTargets.push(...page.targets);
			}
			setState((current) =>
				nextSupervisionState(current, {
					attempts,
					operationReadbacks,
					shutdown,
					shutdownOutcomeUnknown,
					shutdownTargets,
					error: null,
				}),
			);
		} catch (error) {
			if (String(error).includes("cursor")) {
				applicationAttemptCursor.current = null;
			}
			setState((current) =>
				nextSupervisionState(current, {
					...current,
					error: String(error),
				}),
			);
		}
	}, []);

	useEffect(() => {
		applicationAttemptCursor.current = null;
		void refresh();
		const interval = globalThis.setInterval(() => void refresh(), 2_000);
		return () => globalThis.clearInterval(interval);
	}, [refresh]);

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
					shutdown_id: state.shutdown.shutdown_id,
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

	return { state, refresh, retryShutdownTarget, retryQuit };
}
