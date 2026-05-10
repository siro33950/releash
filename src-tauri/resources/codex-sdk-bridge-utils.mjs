export const SUPPORTED_CODEX_MODELS = [
	{ value: "gpt-5.4", displayName: "GPT-5.4" },
	{ value: "gpt-5.3-codex", displayName: "GPT-5.3 Codex" },
	{ value: "gpt-5.2-codex", displayName: "GPT-5.2 Codex" },
	{ value: "gpt-5-codex", displayName: "GPT-5 Codex" },
	{ value: "o3", displayName: "o3" },
];

export function approvalPolicyFromPermissionMode(permissionMode) {
	switch (permissionMode) {
		case "plan":
		case "bypassPermissions":
		case "acceptEdits":
			return "never";
		case "default":
			return "on-request";
		default:
			return "never";
	}
}

export function sandboxModeFromPermissionMode(permissionMode) {
	switch (permissionMode) {
		case "plan":
			return "read-only";
		case "bypassPermissions":
			return "danger-full-access";
		default:
			return "workspace-write";
	}
}

export function createThreadOptions({
	cwd,
	modelId,
	permissionMode,
	skipGitRepoCheck = false,
}) {
	return {
		workingDirectory: cwd,
		skipGitRepoCheck,
		approvalPolicy: approvalPolicyFromPermissionMode(permissionMode),
		sandboxMode: sandboxModeFromPermissionMode(permissionMode),
		...(modelId ? { model: modelId } : {}),
	};
}

export function textDeltaForItem(item, previousText = "") {
	if (!item || item.type !== "agent_message") return null;
	const nextText = item.text ?? "";
	if (!nextText) return null;
	if (nextText.startsWith(previousText)) {
		return nextText.slice(previousText.length) || null;
	}
	return nextText;
}

export function codexEventToBridgeMessages(event, state) {
	switch (event.type) {
		case "thread.started":
			state.threadId = event.thread_id;
			return [{ type: "session_ready", session_id: event.thread_id }];
		case "turn.completed":
			return [
				{
					type: "result",
					session_id: state.threadId ?? null,
					modelUsage: usageToModelUsage(event.usage),
				},
				{
					type: "turn_complete",
					session_id: state.threadId ?? null,
					exit_code: 0,
				},
			];
		case "turn.failed":
			return [
				{
					type: "error",
					message: event.error?.message ?? "Codex turn failed",
					clear_session_id: state.clearSessionIdOnFailure,
				},
				{
					type: "turn_complete",
					session_id: state.threadId ?? null,
					exit_code: 1,
				},
			];
		case "error":
			return [
				{
					type: "error",
					message: event.message ?? "Codex stream error",
					clear_session_id: state.clearSessionIdOnFailure,
				},
				{
					type: "turn_complete",
					session_id: state.threadId ?? null,
					exit_code: 1,
				},
			];
		case "item.started":
		case "item.updated":
		case "item.completed":
			return convertItemEvent(event, state);
		default:
			return [];
	}
}

function usageToModelUsage(usage) {
	if (!usage) return {};
	return {
		codex: {
			inputTokens: usage.input_tokens ?? 0,
			outputTokens: usage.output_tokens ?? 0,
		},
	};
}

function convertItemEvent(event, state) {
	const item = event.item;
	if (!item) return [];

	switch (item.type) {
		case "agent_message":
			return convertAgentMessage(event.type, item, state);
		case "reasoning":
			return textItemDelta(state, item.id, item.text, "thinking_delta", "thinking");
		case "command_execution":
			return convertCommandExecution(item, state);
		case "mcp_tool_call":
			return convertMcpToolCall(item, state);
		case "file_change":
			return convertFileChange(item, state);
		case "todo_list":
			return convertTodoList(item, state);
		case "error":
			return [{ type: "error", message: item.message ?? "Codex item error" }];
		default:
			return [];
	}
}

function convertAgentMessage(eventType, item, state) {
	const textByItemId = ensureTextByItemId(state);
	const previous = textByItemId.get(item.id) ?? "";
	const delta = textDeltaForItem(item, previous);
	textByItemId.set(item.id, item.text ?? "");

	const messages = [];
	if (delta) {
		messages.push(...textLikeDelta("text_delta", "text", delta));
	}
	if (eventType === "item.completed") {
		messages.push({
			type: "assistant",
			message: {
				role: "assistant",
				content: [{ type: "text", text: item.text ?? "" }],
			},
		});
	}
	return messages;
}

function ensureTextByItemId(state) {
	if (!state.textByItemId) {
		state.textByItemId = state.itemText ?? new Map();
	}
	state.itemText = state.textByItemId;
	return state.textByItemId;
}

function ensureSeenToolUses(state) {
	if (!state.seenToolUses) {
		state.seenToolUses = new Set();
	}
	return state.seenToolUses;
}

function ensureCompletedResults(state) {
	if (!state.completedResults) {
		state.completedResults = new Set();
	}
	return state.completedResults;
}

function textItemDelta(state, itemId, nextText, deltaType, field) {
	if (!itemId) return textLikeDelta(deltaType, field, nextText);
	const textByItemId = ensureTextByItemId(state);
	const previous = textByItemId.get(itemId) ?? "";
	const text = nextText ?? "";
	if (!text) return [];
	const delta = text.startsWith(previous) ? text.slice(previous.length) : text;
	textByItemId.set(itemId, text);
	return textLikeDelta(deltaType, field, delta);
}

function textLikeDelta(deltaType, field, text) {
	if (!text) return [];
	return [
		{
			type: "stream_event",
			event: {
				type: "content_block_delta",
				delta: {
					type: deltaType,
					[field]: text,
				},
			},
		},
	];
}

function toolUseMessage(id, name, input) {
	return {
		type: "assistant",
		message: {
			role: "assistant",
			content: [
				{
					type: "tool_use",
					id,
					name,
					input,
				},
			],
		},
	};
}

function toolResultMessage(id, content, isError) {
	return {
		type: "user",
		message: {
			role: "user",
			content: [
				{
					type: "tool_result",
					tool_use_id: id,
					content,
					is_error: isError,
				},
			],
		},
	};
}

function maybeToolUse(state, item, buildMessage) {
	const seenToolUses = ensureSeenToolUses(state);
	if (seenToolUses.has(item.id)) return [];
	seenToolUses.add(item.id);
	return [buildMessage()];
}

function maybeTerminalToolResult(state, item, buildMessage) {
	if (item.status !== "completed" && item.status !== "failed") return [];
	const completedResults = ensureCompletedResults(state);
	if (completedResults.has(item.id)) return [];
	completedResults.add(item.id);
	return [buildMessage()];
}

function convertCommandExecution(item, state) {
	return [
		...maybeToolUse(state, item, () =>
			toolUseMessage(item.id, "CodexCommand", {
				command: item.command,
				status: item.status,
			}),
		),
		...maybeTerminalToolResult(state, item, () =>
			toolResultMessage(
				item.id,
				item.aggregated_output ?? "",
				item.status === "failed",
			),
		),
	];
}

function convertMcpToolCall(item, state) {
	return [
		...maybeToolUse(state, item, () =>
			toolUseMessage(item.id, `${item.server}.${item.tool}`, item.arguments ?? {}),
		),
		...maybeTerminalToolResult(state, item, () =>
			toolResultMessage(
				item.id,
				JSON.stringify(item.result ?? item.error ?? {}),
				item.status === "failed",
			),
		),
	];
}

function convertFileChange(item, state) {
	return maybeToolUse(state, item, () =>
		toolUseMessage(item.id, "CodexFileChange", {
			status: item.status,
			changes: item.changes ?? [],
		}),
	);
}

function convertTodoList(item, state) {
	const text = (item.items ?? [])
		.map((todo) => `${todo.completed ? "[x]" : "[ ]"} ${todo.text}`)
		.join("\n");
	return textItemDelta(state, item.id, text, "thinking_delta", "thinking");
}
