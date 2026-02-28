import { describe, expect, it } from "vitest";
import { parseStreamJson } from "./parseStreamJson";

describe("parseStreamJson", () => {
	it("should return empty string for empty input", () => {
		expect(parseStreamJson("")).toBe("");
	});

	it("should pass through non-JSON text with ANSI stripped", () => {
		expect(parseStreamJson("\u001B[32mhello\u001B[0m")).toBe("hello");
	});

	it("should parse assistant text content", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [{ type: "text", text: "Found 2 issues." }],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("Found 2 issues.");
	});

	it("should parse tool_use with file_path", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{
						type: "tool_use",
						name: "Read",
						input: { file_path: "src/app.ts" },
					},
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("[Read] src/app.ts");
	});

	it("should parse tool_use with command", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{
						type: "tool_use",
						name: "Bash",
						input: { command: "git diff --cached" },
					},
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"[Bash] git diff --cached",
		);
	});

	it("should parse tool_use with pattern", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{
						type: "tool_use",
						name: "Grep",
						input: { pattern: "TODO" },
					},
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("[Grep] TODO");
	});

	it("should parse tool_use with generic input", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{
						type: "tool_use",
						name: "mcp__releash__post_review_comment",
						input: { file: "src/app.ts", line: 42 },
					},
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"[mcp__releash__post_review_comment] file: src/app.ts, line: 42",
		);
	});

	it("should parse tool_use with no input", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [{ type: "tool_use", name: "TaskList", input: {} }],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("[TaskList]");
	});

	it("should parse result event", () => {
		const event = {
			type: "result",
			subtype: "success",
			result: "Review complete.",
			session_id: "abc123",
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("Review complete.");
	});

	it("should skip system events", () => {
		const event = { type: "system", subtype: "init", session_id: "abc" };
		expect(parseStreamJson(JSON.stringify(event))).toBe("");
	});

	it("should skip user (tool_result) events", () => {
		const event = {
			type: "user",
			message: {
				role: "user",
				content: [
					{ type: "tool_result", tool_use_id: "t1", content: "file data..." },
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("");
	});

	it("should handle multiple content blocks in one message", () => {
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{ type: "text", text: "Let me read the file." },
					{
						type: "tool_use",
						name: "Read",
						input: { file_path: "src/main.ts" },
					},
				],
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"Let me read the file.\n[Read] src/main.ts",
		);
	});

	it("should handle multiple NDJSON lines", () => {
		const lines = [
			JSON.stringify({
				type: "assistant",
				message: {
					role: "assistant",
					content: [{ type: "text", text: "Analyzing..." }],
				},
			}),
			JSON.stringify({
				type: "assistant",
				message: {
					role: "assistant",
					content: [
						{
							type: "tool_use",
							name: "Read",
							input: { file_path: "src/foo.ts" },
						},
					],
				},
			}),
			JSON.stringify({ type: "user", message: { role: "user", content: [] } }),
			JSON.stringify({
				type: "result",
				subtype: "success",
				result: "Done.",
			}),
		].join("\n");

		expect(parseStreamJson(lines)).toBe(
			"Analyzing...\n[Read] src/foo.ts\nDone.",
		);
	});

	it("should handle mixed JSON and non-JSON lines", () => {
		const lines = [
			"some startup text",
			JSON.stringify({
				type: "assistant",
				message: {
					role: "assistant",
					content: [{ type: "text", text: "Hello" }],
				},
			}),
		].join("\n");

		expect(parseStreamJson(lines)).toBe("some startup text\nHello");
	});

	it("should parse Codex item.completed with agent_message", () => {
		const event = {
			type: "item.completed",
			item: {
				id: "msg_1",
				type: "agent_message",
				text: "Repo contains docs and src directories.",
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"Repo contains docs and src directories.",
		);
	});

	it("should parse Codex item.started with command_execution", () => {
		const event = {
			type: "item.started",
			item: {
				id: "cmd_1",
				type: "command_execution",
				command: "bash -lc ls",
				status: "in_progress",
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"[command] bash -lc ls",
		);
	});

	it("should parse Codex item.completed with mcp_tool_call", () => {
		const event = {
			type: "item.completed",
			item: {
				id: "mcp_1",
				type: "mcp_tool_call",
				name: "mcp__releash__post_review_comment",
				arguments: { file: "src/app.ts", line: 42 },
			},
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe(
			"[mcp__releash__post_review_comment] file: src/app.ts, line: 42",
		);
	});

	it("should skip Codex thread.started event", () => {
		const event = { type: "thread.started", thread_id: "t_1" };
		expect(parseStreamJson(JSON.stringify(event))).toBe("");
	});

	it("should skip Codex turn.started event", () => {
		const event = { type: "turn.started" };
		expect(parseStreamJson(JSON.stringify(event))).toBe("");
	});

	it("should skip Codex turn.completed event", () => {
		const event = {
			type: "turn.completed",
			usage: { input_tokens: 24763, output_tokens: 122 },
		};
		expect(parseStreamJson(JSON.stringify(event))).toBe("");
	});

	it("should truncate long generic input values", () => {
		const longValue = "x".repeat(200);
		const event = {
			type: "assistant",
			message: {
				role: "assistant",
				content: [
					{
						type: "tool_use",
						name: "CustomTool",
						input: { data: longValue },
					},
				],
			},
		};
		const result = parseStreamJson(JSON.stringify(event));
		expect(result).toBe(`[CustomTool] data: ${"x".repeat(80)}`);
	});
});
