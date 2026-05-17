import { describe, expect, it } from "vitest";
import {
	codexEventToBridgeMessages,
	createThreadOptions,
	textDeltaForItem,
} from "./codex-sdk-bridge-utils.mjs";

describe("codex-sdk-bridge-utils", () => {
	it("passes through Codex flags supplied by Rust without translation", () => {
		// Rust 側のバックエンド変換層が抽象モード → approvalPolicy / sandboxMode を解決し、
		// このユーティリティは値をそのまま @openai/codex-sdk に渡す。
		expect(
			createThreadOptions({
				cwd: "/repo",
				modelId: "gpt-5.4",
				approvalPolicy: "never",
				sandboxMode: "workspace-write",
			}),
		).toEqual({
			workingDirectory: "/repo",
			skipGitRepoCheck: false,
			approvalPolicy: "never",
			sandboxMode: "workspace-write",
			model: "gpt-5.4",
		});
	});

	it("passes through read-only sandbox flags from Rust", () => {
		expect(
			createThreadOptions({
				cwd: "/repo",
				modelId: null,
				approvalPolicy: "never",
				sandboxMode: "read-only",
			}),
		).toEqual({
			workingDirectory: "/repo",
			skipGitRepoCheck: false,
			approvalPolicy: "never",
			sandboxMode: "read-only",
		});
	});

	it("passes through danger-full-access sandbox flags from Rust", () => {
		expect(
			createThreadOptions({
				cwd: "/repo",
				modelId: null,
				approvalPolicy: "never",
				sandboxMode: "danger-full-access",
			}),
		).toEqual({
			workingDirectory: "/repo",
			skipGitRepoCheck: false,
			approvalPolicy: "never",
			sandboxMode: "danger-full-access",
		});
	});

	it("converts thread.started to session_ready", () => {
		const state = { itemText: new Map() };
		expect(
			codexEventToBridgeMessages(
				{ type: "thread.started", thread_id: "thread-1" },
				state,
			),
		).toEqual([{ type: "session_ready", session_id: "thread-1" }]);
		expect(state.threadId).toBe("thread-1");
	});

	it("calculates agent message deltas", () => {
		const item = { id: "i1", type: "agent_message", text: "hello world" };
		expect(textDeltaForItem(item, "hello")).toBe(" world");
		expect(textDeltaForItem(item, "hello world")).toBeNull();
	});

	it("converts completed agent message to text delta and assistant message", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		const messages = codexEventToBridgeMessages(
			{
				type: "item.completed",
				item: { id: "i1", type: "agent_message", text: "done" },
			},
			state,
		);
		expect(messages[0]).toMatchObject({
			type: "stream_event",
			event: { delta: { type: "text_delta", text: "done" } },
		});
		expect(messages[1]).toEqual({
			type: "assistant",
			message: {
				role: "assistant",
				content: [{ type: "text", text: "done" }],
			},
		});
	});

	it("converts failed turn to error and non-zero completion", () => {
		const messages = codexEventToBridgeMessages(
			{ type: "turn.failed", error: { message: "bad auth" } },
			{
				threadId: "thread-1",
				itemText: new Map(),
				clearSessionIdOnFailure: true,
			},
		);
		expect(messages).toEqual([
			{ type: "error", message: "bad auth", clear_session_id: true },
			{ type: "turn_complete", session_id: "thread-1", exit_code: 1 },
		]);
	});

	it("converts command item.started to one tool_use without permission events", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		const messages = codexEventToBridgeMessages(
			{
				type: "item.started",
				item: {
					id: "cmd-1",
					type: "command_execution",
					command: "pnpm test",
					aggregated_output: "",
					status: "in_progress",
				},
			},
			state,
		);

		expect(messages).toEqual([
			{
				type: "assistant",
				message: {
					role: "assistant",
					content: [
						{
							type: "tool_use",
							id: "cmd-1",
							name: "CodexCommand",
							input: { command: "pnpm test", status: "in_progress" },
						},
					],
				},
			},
		]);
		expect(messages.some((message) => message.type === "permission_request")).toBe(
			false,
		);
	});

	it("does not duplicate command tool_use or emit in-progress tool_result", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		codexEventToBridgeMessages(
			{
				type: "item.started",
				item: {
					id: "cmd-1",
					type: "command_execution",
					command: "pnpm test",
					aggregated_output: "",
					status: "in_progress",
				},
			},
			state,
		);

		const messages = codexEventToBridgeMessages(
			{
				type: "item.updated",
				item: {
					id: "cmd-1",
					type: "command_execution",
					command: "pnpm test",
					aggregated_output: "running",
					status: "in_progress",
				},
			},
			state,
		);

		expect(messages).toEqual([]);
	});

	it("emits command terminal tool_result only once", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		codexEventToBridgeMessages(
			{
				type: "item.started",
				item: {
					id: "cmd-1",
					type: "command_execution",
					command: "pnpm test",
					aggregated_output: "",
					status: "in_progress",
				},
			},
			state,
		);

		const completed = {
			id: "cmd-1",
			type: "command_execution",
			command: "pnpm test",
			aggregated_output: "ok",
			status: "completed",
		};
		const first = codexEventToBridgeMessages(
			{ type: "item.updated", item: completed },
			state,
		);
		const second = codexEventToBridgeMessages(
			{ type: "item.completed", item: completed },
			state,
		);

		expect(first).toEqual([
			{
				type: "user",
				message: {
					role: "user",
					content: [
						{
							type: "tool_result",
							tool_use_id: "cmd-1",
							content: "ok",
							is_error: false,
						},
					],
				},
			},
		]);
		expect(second).toEqual([]);
	});

	it("deduplicates MCP tool calls and emits one terminal result", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		const started = codexEventToBridgeMessages(
			{
				type: "item.started",
				item: {
					id: "mcp-1",
					type: "mcp_tool_call",
					server: "fs",
					tool: "read",
					arguments: { path: "README.md" },
					status: "in_progress",
				},
			},
			state,
		);
		const completed = codexEventToBridgeMessages(
			{
				type: "item.completed",
				item: {
					id: "mcp-1",
					type: "mcp_tool_call",
					server: "fs",
					tool: "read",
					arguments: { path: "README.md" },
					result: { content: [], structured_content: { ok: true } },
					status: "completed",
				},
			},
			state,
		);

		expect(started).toHaveLength(1);
		expect(started[0].message.content[0]).toMatchObject({
			type: "tool_use",
			id: "mcp-1",
			name: "fs.read",
		});
		expect(completed).toHaveLength(1);
		expect(completed[0].message.content[0]).toMatchObject({
			type: "tool_result",
			tool_use_id: "mcp-1",
			is_error: false,
		});
		expect(
			codexEventToBridgeMessages(
				{
					type: "item.completed",
					item: {
						id: "mcp-1",
						type: "mcp_tool_call",
						server: "fs",
						tool: "read",
						arguments: { path: "README.md" },
						result: { content: [], structured_content: { ok: true } },
						status: "completed",
					},
				},
				state,
			),
		).toEqual([]);
	});

	it("emits reasoning deltas from cumulative item text", () => {
		const state = { threadId: "thread-1", itemText: new Map() };
		const first = codexEventToBridgeMessages(
			{
				type: "item.updated",
				item: { id: "reason-1", type: "reasoning", text: "plan" },
			},
			state,
		);
		const second = codexEventToBridgeMessages(
			{
				type: "item.updated",
				item: { id: "reason-1", type: "reasoning", text: "plan next" },
			},
			state,
		);

		expect(first[0]).toMatchObject({
			type: "stream_event",
			event: { delta: { type: "thinking_delta", thinking: "plan" } },
		});
		expect(second[0]).toMatchObject({
			type: "stream_event",
			event: { delta: { type: "thinking_delta", thinking: " next" } },
		});
	});
});
