import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { getErrorMessage } from "@/lib/errorMessage";

interface PendingApplicationAttempt {
	kind: "application_quit";
	caller_request_id: string;
	operation_id: string | null;
	resolution: "pending" | "accepted" | "rejected_before_commit";
}

interface PendingApplicationAttemptPage {
	entries: PendingApplicationAttempt[];
	next_cursor: string | null;
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
		| { type: "exit_coupled_outcome_unknown"; shutdown_id: string }
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

interface ApplicationShutdownSupervisionState {
	attempts: PendingApplicationAttempt[];
	shutdown: ShutdownProjection | null;
	shutdownOutcomeUnknown: ShutdownOutcomeUnknown | null;
	shutdownTargets: ShutdownTargetCapability[];
	error: string | null;
}

const EMPTY_STATE: ApplicationShutdownSupervisionState = {
	attempts: [],
	shutdown: null,
	shutdownOutcomeUnknown: null,
	shutdownTargets: [],
	error: null,
};

const QUIT_ATTEMPT_STORAGE_KEY = "releash:application-quit-attempt:v1";
const MAX_ATTEMPT_PAGES = 16;
const MAX_ATTEMPTS = 512;

interface QuitAttemptSnapshot {
	requestId: string;
	intent: { type: "exit" | "restart"; code: number };
}

function sameState(
	current: ApplicationShutdownSupervisionState,
	next: ApplicationShutdownSupervisionState,
) {
	return JSON.stringify(current) === JSON.stringify(next) ? current : next;
}

function isInvalidPendingAttemptCursor(
	error: unknown,
	cursor: string | null,
): boolean {
	return (
		cursor !== null &&
		typeof error === "object" &&
		error !== null &&
		"type" in error &&
		error.type === "invalid_request"
	);
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
		return { requestId: saved, intent: { type: "exit", code: 0 } };
	}
}

async function loadAttempts(
	initialCursor: string | null,
	onInvalidCursor: () => void,
) {
	const entries: PendingApplicationAttempt[] = [];
	let cursor = initialCursor;
	for (let page = 0; page < MAX_ATTEMPT_PAGES; page += 1) {
		let result: PendingApplicationAttemptPage;
		try {
			result = await invoke<PendingApplicationAttemptPage>(
				"list_pending_application_attempts",
				{ limit: 32, cursor },
			);
		} catch (error) {
			if (isInvalidPendingAttemptCursor(error, cursor)) onInvalidCursor();
			throw error;
		}
		entries.push(...result.entries);
		cursor = result.next_cursor;
		if (cursor === null || entries.length >= MAX_ATTEMPTS) break;
	}
	return { entries: entries.slice(0, MAX_ATTEMPTS), nextCursor: cursor };
}

async function redispatchPendingQuit(attempts: PendingApplicationAttempt[]) {
	const snapshot = loadQuitAttempt();
	if (
		!snapshot ||
		!attempts.some(
			(attempt) =>
				attempt.resolution === "pending" &&
				attempt.caller_request_id === snapshot.requestId,
		)
	) {
		return;
	}
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

async function acknowledgeAcceptedAttempts(
	attempts: PendingApplicationAttempt[],
) {
	for (const attempt of attempts) {
		if (attempt.resolution !== "accepted") continue;
		const operationId = attempt.operation_id ?? attempt.caller_request_id;
		await invoke("get_application_quit_operation", { operationId });
		await invoke("acknowledge_application_attempt", {
			callerRequestId: attempt.caller_request_id,
		});
	}
}

export function useApplicationShutdownSupervision() {
	const [state, setState] = useState(EMPTY_STATE);
	const attemptCursor = useRef<string | null>(null);

	const refresh = useCallback(async () => {
		try {
			const attemptPagePromise = loadAttempts(attemptCursor.current, () => {
				attemptCursor.current = null;
			});
			const [attemptPage, shutdownResult] = await Promise.all([
				attemptPagePromise,
				invoke<CurrentShutdownResult>("get_application_shutdown"),
			]);
			attemptCursor.current = attemptPage.nextCursor;
			await redispatchPendingQuit(attemptPage.entries);
			await acknowledgeAcceptedAttempts(attemptPage.entries);

			const shutdown =
				shutdownResult.type === "current" ? shutdownResult.plan : null;
			const shutdownOutcomeUnknown =
				shutdownResult.type === "outcome_unknown"
					? {
							operation_id: shutdownResult.operation_id,
							intent: shutdownResult.intent,
						}
					: null;
			const shutdownTargets = shutdown
				? (
						await invoke<ShutdownPlanPage>("get_shutdown_plan", {
							shutdownId: shutdown.shutdown_id,
							limit: 128,
							cursor: null,
						})
					).targets
				: [];
			setState((current) =>
				sameState(current, {
					attempts: attemptPage.entries,
					shutdown,
					shutdownOutcomeUnknown,
					shutdownTargets,
					error: null,
				}),
			);
		} catch (error) {
			setState((current) =>
				sameState(current, { ...current, error: getErrorMessage(error) }),
			);
		}
	}, []);

	useEffect(() => {
		attemptCursor.current = null;
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
