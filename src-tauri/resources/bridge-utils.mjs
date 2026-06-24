/**
 * Build the systemPrompt option for Claude Agent SDK.
 * Transforms a plain string into the preset+append format
 * that preserves Claude Code's built-in instructions.
 *
 * @param {string|undefined|null} systemPrompt
 * @returns {object} Spread-ready object (empty if no systemPrompt)
 */
export function buildSystemPromptOption(systemPrompt) {
	if (!systemPrompt) return {};
	return {
		systemPrompt: {
			type: "preset",
			preset: "claude_code",
			append: systemPrompt,
		},
	};
}

/**
 * Decide whether the bridge should keep its process alive after the SDK
 * iterator ends. Only a successful turn completion is reusable by Rust; failed
 * results move the Rust bridge state to Crashed, so the JS process must exit
 * instead of continuing without an owner.
 *
 * @param {{ closed: boolean, turnExitCode: number|null }} state
 * @returns {boolean}
 */
export function shouldContinueBridgeLoopAfterQueryEnd(state) {
	return !state.closed && state.turnExitCode === 0;
}

/**
 * Decide whether an incoming user prompt can be delivered to the prompt
 * resolver currently owned by the active SDK query. Once a query has produced
 * a result, later prompts must wait for the next query so they are not consumed
 * by an iterator that is already finishing.
 *
 * @param {{ hasPendingPromptResolver: boolean, completedResultForCurrentQuery: boolean }} state
 * @returns {boolean}
 */
export function shouldResolvePromptForCurrentQuery(state) {
	return (
		state.hasPendingPromptResolver && !state.completedResultForCurrentQuery
	);
}

export function createQueryInitTelemetryState() {
	return {
		startedAtMs: null,
		emitted: false,
	};
}

export function markQueryInitTelemetryStarted(state, nowMs) {
	state.startedAtMs = Number.isFinite(nowMs) ? nowMs : null;
	state.emitted = false;
}

export function consumeQueryInitTelemetryMessage(state, nowMs) {
	if (state.emitted) return null;
	state.emitted = true;
	const endedAtMs = Number.isFinite(nowMs) ? nowMs : state.startedAtMs;
	const startedAtMs = Number.isFinite(state.startedAtMs)
		? state.startedAtMs
		: endedAtMs;
	return {
		type: "telemetry",
		metric: "query_init",
		duration_ms: Math.max(0, endedAtMs - startedAtMs),
	};
}

/**
 * Build a bridge turn completion message. The interrupted flag is explicit so
 * Rust does not need to infer user interrupts from exit codes.
 *
 * @param {{ sessionId?: string|null, exitCode: number, interrupted?: boolean, turnToken?: string|null }} state
 * @returns {object}
 */
export function buildTurnCompleteMessage(state) {
	const message = {
		type: "turn_complete",
		session_id: state.sessionId || null,
		exit_code: state.exitCode,
	};
	if (state.interrupted) {
		message.interrupted = true;
	}
	if (state.turnToken) {
		message.turn_token = state.turnToken;
	}
	return message;
}

/**
 * Build the completion state for an SDK result message. If the turn was already
 * aborted, interruption wins over a successful result so the interrupted SDK
 * session never becomes the clean resume point.
 *
 * @param {{ sessionId?: string|null, currentSessionId?: string|null, hasErrors: boolean, wasAborted: boolean, turnToken?: string|null }} state
 * @returns {{ message: object, exitCode: number, completedSessionIdForResume: string|null }}
 */
export function buildResultTurnCompletion(state) {
	const exitCode = state.wasAborted ? 0 : state.hasErrors ? 1 : 0;
	const completedSessionIdForResume =
		!state.wasAborted && exitCode === 0
			? state.sessionId || state.currentSessionId || null
			: null;
	return {
		message: buildTurnCompleteMessage({
			sessionId: state.sessionId || null,
			exitCode,
			interrupted: state.wasAborted,
			turnToken: state.turnToken,
		}),
		exitCode,
		completedSessionIdForResume,
	};
}

/**
 * Abort rolls the SDK resume point back to the last cleanly completed result.
 * If no clean result exists, the next query must start a fresh SDK session.
 *
 * @param {{ lastResultSessionId?: string|null }} state
 * @returns {string|null}
 */
export function rollbackResumeSessionIdAfterInterrupt(state) {
	return state.lastResultSessionId || null;
}

/**
 * Echo the Rust-issued turn token on SDK events generated for that turn.
 *
 * @param {object} message
 * @param {string|null|undefined} turnToken
 * @returns {object}
 */
export function withTurnToken(message, turnToken) {
	if (!turnToken) return message;
	return { ...message, turn_token: turnToken };
}
