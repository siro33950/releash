import { stripAnsi } from "./stripAnsi";

interface ContentBlock {
	type: string;
	text?: string;
	thinking?: string;
	name?: string;
	input?: Record<string, unknown>;
}

interface CodexItem {
	id?: string;
	type?: string;
	text?: string;
	command?: string;
	status?: string;
	name?: string;
	arguments?: Record<string, unknown>;
}

interface StreamEvent {
	type: string;
	subtype?: string;
	message?: {
		role: string;
		content: ContentBlock[];
	};
	result?: string;
	item?: CodexItem;
}

/**
 * Parse Claude Code `--output-format stream-json` NDJSON output
 * into human-readable text. Non-JSON lines fall through to ANSI stripping.
 */
export function parseStreamJson(raw: string): string {
	const lines = raw.split("\n");
	const parts: string[] = [];

	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed) continue;

		try {
			const event = JSON.parse(trimmed) as StreamEvent;
			const formatted = formatEvent(event);
			if (formatted) parts.push(formatted);
		} catch {
			parts.push(stripAnsi(trimmed));
		}
	}

	return parts.join("\n");
}

function formatEvent(event: StreamEvent): string {
	switch (event.type) {
		case "assistant":
			return formatAssistant(event.message?.content ?? []);
		case "user":
			return "";
		case "result":
			return event.result ?? "";
		case "item.completed":
			return formatCodexItem(event.item);
		case "item.started":
			return formatCodexItem(event.item);
		case "thread.started":
		case "turn.started":
		case "turn.completed":
			return "";
		default:
			return "";
	}
}

function formatCodexItem(item: CodexItem | undefined): string {
	if (!item) return "";

	if (item.type === "agent_message" && item.text) {
		return item.text;
	}

	if (item.type === "command_execution" && item.command) {
		return `[command] ${item.command}`;
	}

	if (item.type === "mcp_tool_call" && item.name) {
		const args = item.arguments ?? {};
		const keys = Object.keys(args);
		if (keys.length === 0) return `[${item.name}]`;
		const summary = keys
			.slice(0, 3)
			.map((k) => `${k}: ${String(args[k]).slice(0, 80)}`)
			.join(", ");
		return `[${item.name}] ${summary}`;
	}

	return "";
}

function formatAssistant(content: ContentBlock[]): string {
	return content
		.map((block) => {
			if (block.type === "text") return block.text ?? "";
			if (block.type === "tool_use") return formatToolUse(block);
			return "";
		})
		.filter(Boolean)
		.join("\n");
}

function formatToolUse(block: ContentBlock): string {
	const name = block.name ?? "unknown";
	const input = block.input ?? {};

	if (input.file_path) return `[${name}] ${input.file_path}`;
	if (input.command) return `[${name}] ${input.command}`;
	if (input.pattern) return `[${name}] ${input.pattern}`;
	if (input.query) return `[${name}] ${input.query}`;

	const keys = Object.keys(input);
	if (keys.length === 0) return `[${name}]`;

	const summary = keys
		.slice(0, 3)
		.map((k) => `${k}: ${String(input[k]).slice(0, 80)}`)
		.join(", ");
	return `[${name}] ${summary}`;
}
