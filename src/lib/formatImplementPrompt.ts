/**
 * Generate a prompt for the AI coding agent to implement changes
 * discussed in a thread. The prompt instructs the agent to use
 * the MCP `get_thread` tool to retrieve the full thread context.
 */
export function formatImplementPrompt(threadId: string): string {
	return [
		`Please implement the changes discussed in thread "${threadId}".`,
		"",
		"Steps:",
		`1. Call the \`get_thread\` tool with thread_id="${threadId}" to read the full discussion.`,
		"2. Understand the requested changes from the thread entries.",
		"3. Implement the changes in the relevant files.",
		"4. After implementation, call `resolve_thread` to mark the thread as resolved.",
	].join("\n");
}
