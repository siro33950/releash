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
