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
