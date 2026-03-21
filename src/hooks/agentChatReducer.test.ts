import { describe, expect, it } from "vitest";
import type { ChatMessage, ChatSession } from "@/types/session";
import type { AgentChatState } from "./agentChatReducer";
import { INITIAL_STATE, reducer } from "./agentChatReducer";

function makeSession(overrides?: Partial<ChatSession>): ChatSession {
	return {
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		...overrides,
	};
}

function makeMessage(overrides?: Partial<ChatMessage>): ChatMessage {
	return {
		id: "m1",
		role: "human",
		parts: [{ type: "text", content: "hello" }],
		timestamp: 1000,
		...overrides,
	};
}

describe("agentChatReducer", () => {
	it("INITIAL_STATE has expected shape", () => {
		expect(INITIAL_STATE).toEqual({
			sessions: [],
			sessionOrder: [],
			closedSessions: [],
			activeSession: null,
			streamingSessionIds: [],
			error: null,
			permissionMode: "acceptEdits",
			userPermissionMode: "acceptEdits",
			pendingPermissions: {},
		});
	});

	describe("SET_SESSIONS", () => {
		it("replaces sessions list and builds sessionOrder", () => {
			const sessions = [
				{
					id: "s1",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1000,
					updatedAt: 1000,
					firstMessage: "hello",
					messageCount: 1,
				},
			];
			const next = reducer(INITIAL_STATE, {
				type: "SET_SESSIONS",
				sessions,
			});
			expect(next.sessions).toBe(sessions);
			expect(next.sessionOrder).toEqual(["s1"]);
		});

		it("preserves existing order and appends new sessions", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionOrder: ["s2", "s1"],
				sessions: [
					{
						id: "s1",
						worktreePath: "/repo",
						state: "idle" as const,
						createdAt: 1000,
						updatedAt: 1000,
						firstMessage: "first",
						messageCount: 1,
					},
					{
						id: "s2",
						worktreePath: "/repo",
						state: "idle" as const,
						createdAt: 900,
						updatedAt: 900,
						firstMessage: "second",
						messageCount: 1,
					},
				],
			};
			const newSessions = [
				...state.sessions,
				{
					id: "s3",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1100,
					updatedAt: 1100,
					firstMessage: "third",
					messageCount: 1,
				},
			];
			const next = reducer(state, {
				type: "SET_SESSIONS",
				sessions: newSessions,
			});
			expect(next.sessionOrder).toEqual(["s2", "s1", "s3"]);
		});

		it("removes deleted session IDs from order", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionOrder: ["s1", "s2", "s3"],
			};
			const sessions = [
				{
					id: "s1",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1000,
					updatedAt: 1000,
					firstMessage: "first",
					messageCount: 1,
				},
				{
					id: "s3",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1100,
					updatedAt: 1100,
					firstMessage: "third",
					messageCount: 1,
				},
			];
			const next = reducer(state, { type: "SET_SESSIONS", sessions });
			expect(next.sessionOrder).toEqual(["s1", "s3"]);
		});
	});

	describe("REORDER_SESSIONS", () => {
		it("replaces sessionOrder", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionOrder: ["s1", "s2", "s3"],
			};
			const next = reducer(state, {
				type: "REORDER_SESSIONS",
				sessionOrder: ["s3", "s1", "s2"],
			});
			expect(next.sessionOrder).toEqual(["s3", "s1", "s2"]);
		});
	});

	describe("SET_CLOSED_SESSIONS", () => {
		it("replaces closedSessions list", () => {
			const sessions = [
				{
					id: "s1",
					worktreePath: "/repo",
					state: "closed" as const,
					createdAt: 1000,
					updatedAt: 1000,
					firstMessage: "hello",
					messageCount: 1,
				},
			];
			const next = reducer(INITIAL_STATE, {
				type: "SET_CLOSED_SESSIONS",
				sessions,
			});
			expect(next.closedSessions).toBe(sessions);
		});
	});

	describe("SET_ACTIVE_SESSION", () => {
		it("sets active session and clears error", () => {
			const stateWithError: AgentChatState = {
				...INITIAL_STATE,
				error: "some error",
			};
			const session = makeSession();
			const next = reducer(stateWithError, {
				type: "SET_ACTIVE_SESSION",
				session,
			});
			expect(next.activeSession).toBe(session);
			expect(next.error).toBeNull();
		});

		it("sets active session to null", () => {
			const stateWithSession: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession(),
			};
			const next = reducer(stateWithSession, {
				type: "SET_ACTIVE_SESSION",
				session: null,
			});
			expect(next.activeSession).toBeNull();
		});
	});

	describe("ADD_MESSAGE", () => {
		it("appends message to active session", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession(),
			};
			const msg = makeMessage();
			const next = reducer(state, { type: "ADD_MESSAGE", message: msg });
			expect(next.activeSession?.messages).toHaveLength(1);
			expect(next.activeSession?.messages[0]).toBe(msg);
		});

		it("does nothing when no active session", () => {
			const msg = makeMessage();
			const next = reducer(INITIAL_STATE, {
				type: "ADD_MESSAGE",
				message: msg,
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});

	describe("APPEND_STREAMING", () => {
		it("appends chunk to matching message", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "Hello" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_STREAMING",
				messageId: "m1",
				chunk: " world",
			});
			const textPart = next.activeSession?.messages[0].parts[0];
			expect(textPart?.type).toBe("text");
			expect((textPart as { content: string }).content).toBe("Hello world");
		});

		it("does not modify non-matching messages", () => {
			const msg1 = makeMessage({
				id: "m1",
				parts: [{ type: "text", content: "first" }],
			});
			const msg2 = makeMessage({
				id: "m2",
				role: "agent",
				parts: [{ type: "text", content: "second" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg1, msg2] }),
			};
			const next = reducer(state, {
				type: "APPEND_STREAMING",
				messageId: "m2",
				chunk: "!",
			});
			expect(
				(next.activeSession?.messages[0].parts[0] as { content: string })
					.content,
			).toBe("first");
			expect(
				(next.activeSession?.messages[1].parts[0] as { content: string })
					.content,
			).toBe("second!");
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "APPEND_STREAMING",
				messageId: "m1",
				chunk: "data",
			});
			expect(next).toBe(INITIAL_STATE);
		});

		it("duplicates content when same chunk is appended twice", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const step1 = reducer(state, {
				type: "APPEND_STREAMING",
				messageId: "m1",
				chunk: "hello",
			});
			const step2 = reducer(step1, {
				type: "APPEND_STREAMING",
				messageId: "m1",
				chunk: "hello",
			});
			expect(
				(step2.activeSession?.messages[0].parts[0] as { content: string })
					.content,
			).toBe("hellohello");
		});
	});

	describe("APPEND_THINKING", () => {
		it("appends chunk to thinking part of matching message", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "thinking", content: "Let me" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_THINKING",
				messageId: "m1",
				chunk: " think",
			});
			const thinkingPart = next.activeSession?.messages[0].parts[0];
			expect(thinkingPart?.type).toBe("thinking");
			expect((thinkingPart as { content: string }).content).toBe(
				"Let me think",
			);
		});

		it("creates new thinking part when parts is empty", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_THINKING",
				messageId: "m1",
				chunk: "first chunk",
			});
			const thinkingPart = next.activeSession?.messages[0].parts[0];
			expect(thinkingPart?.type).toBe("thinking");
			expect((thinkingPart as { content: string }).content).toBe("first chunk");
		});

		it("does not modify non-matching messages", () => {
			const msg1 = makeMessage({
				id: "m1",
				parts: [{ type: "text", content: "first" }],
			});
			const msg2 = makeMessage({
				id: "m2",
				role: "agent",
				parts: [{ type: "thinking", content: "a" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg1, msg2] }),
			};
			const next = reducer(state, {
				type: "APPEND_THINKING",
				messageId: "m2",
				chunk: "b",
			});
			expect(next.activeSession?.messages[0].parts[0].type).toBe("text");
			const thinkingPart = next.activeSession?.messages[1].parts[0];
			expect((thinkingPart as { content: string }).content).toBe("ab");
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "APPEND_THINKING",
				messageId: "m1",
				chunk: "data",
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});

	describe("START_STREAMING / STOP_STREAMING", () => {
		it("adds sessionId to streamingSessionIds", () => {
			const next = reducer(INITIAL_STATE, {
				type: "START_STREAMING",
				sessionId: "s1",
			});
			expect(next.streamingSessionIds).toEqual(["s1"]);
		});

		it("does not duplicate sessionId", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				streamingSessionIds: ["s1"],
			};
			const next = reducer(state, {
				type: "START_STREAMING",
				sessionId: "s1",
			});
			expect(next.streamingSessionIds).toEqual(["s1"]);
		});

		it("supports multiple concurrent sessions", () => {
			const step1 = reducer(INITIAL_STATE, {
				type: "START_STREAMING",
				sessionId: "s1",
			});
			const step2 = reducer(step1, {
				type: "START_STREAMING",
				sessionId: "s2",
			});
			expect(step2.streamingSessionIds).toEqual(["s1", "s2"]);
		});

		it("removes sessionId from streamingSessionIds", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				streamingSessionIds: ["s1", "s2"],
			};
			const next = reducer(state, {
				type: "STOP_STREAMING",
				sessionId: "s1",
			});
			expect(next.streamingSessionIds).toEqual(["s2"]);
		});

		it("does nothing when stopping non-streaming session", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				streamingSessionIds: ["s1"],
			};
			const next = reducer(state, {
				type: "STOP_STREAMING",
				sessionId: "s2",
			});
			expect(next.streamingSessionIds).toEqual(["s1"]);
		});
	});

	describe("SET_ERROR", () => {
		it("sets error message", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_ERROR",
				error: "something failed",
			});
			expect(next.error).toBe("something failed");
		});

		it("clears error with null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				error: "old error",
			};
			const next = reducer(state, { type: "SET_ERROR", error: null });
			expect(next.error).toBeNull();
		});
	});

	describe("UPDATE_SESSION_STATE", () => {
		it("updates active session state", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ state: "active" }),
			};
			const next = reducer(state, {
				type: "UPDATE_SESSION_STATE",
				state: "done",
			});
			expect(next.activeSession?.state).toBe("done");
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "UPDATE_SESSION_STATE",
				state: "done",
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});

	describe("APPEND_TOOL_USE", () => {
		it("appends tool_use part to matching message", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_TOOL_USE",
				messageId: "m1",
				tool: "Read",
				input: { file_path: "/src/index.ts" },
				id: "toolu_001",
			});
			expect(next.activeSession?.messages[0].parts).toEqual([
				{
					type: "tool_use",
					tool: "Read",
					input: { file_path: "/src/index.ts" },
					id: "toolu_001",
				},
			]);
		});

		it("adds tool_use part to empty parts array", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_TOOL_USE",
				messageId: "m1",
				tool: "Grep",
				input: { pattern: "TODO" },
				id: "toolu_002",
			});
			expect(next.activeSession?.messages[0].parts).toHaveLength(1);
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "APPEND_TOOL_USE",
				messageId: "m1",
				tool: "Read",
				input: {},
				id: "toolu_001",
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});

	describe("APPEND_TOOL_RESULT", () => {
		it("appends tool_result part to matching message", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [
					{
						type: "tool_use",
						tool: "Read",
						input: { file_path: "/src/index.ts" },
						id: "toolu_001",
					},
				],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_TOOL_RESULT",
				messageId: "m1",
				content: "file contents here",
				isError: false,
			});
			expect(next.activeSession?.messages[0].parts).toHaveLength(2);
			expect(next.activeSession?.messages[0].parts[1]).toEqual({
				type: "tool_result",
				content: "file contents here",
				isError: false,
			});
		});

		it("handles error results", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ messages: [msg] }),
			};
			const next = reducer(state, {
				type: "APPEND_TOOL_RESULT",
				messageId: "m1",
				content: "File not found",
				isError: true,
			});
			expect(next.activeSession?.messages[0].parts[0]).toEqual({
				type: "tool_result",
				content: "File not found",
				isError: true,
			});
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "APPEND_TOOL_RESULT",
				messageId: "m1",
				content: "result",
				isError: false,
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});

	describe("SET_PERMISSION_MODE", () => {
		it("updates permissionMode", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_PERMISSION_MODE",
				mode: "default",
			});
			expect(next.permissionMode).toBe("default");
		});

		it("switches from default to plan", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				permissionMode: "default",
			};
			const next = reducer(state, {
				type: "SET_PERMISSION_MODE",
				mode: "plan",
			});
			expect(next.permissionMode).toBe("plan");
		});
	});

	describe("SET_USER_PERMISSION_MODE", () => {
		it("updates both userPermissionMode and permissionMode", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_USER_PERMISSION_MODE",
				mode: "bypassPermissions",
			});
			expect(next.userPermissionMode).toBe("bypassPermissions");
			expect(next.permissionMode).toBe("bypassPermissions");
		});
	});

	describe("RESTORE_USER_PERMISSION_MODE", () => {
		it("restores permissionMode from userPermissionMode", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				userPermissionMode: "acceptEdits",
				permissionMode: "plan",
			};
			const next = reducer(state, { type: "RESTORE_USER_PERMISSION_MODE" });
			expect(next.permissionMode).toBe("acceptEdits");
		});

		it("falls back to default when userPermissionMode is plan", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				userPermissionMode: "plan",
				permissionMode: "plan",
			};
			const next = reducer(state, { type: "RESTORE_USER_PERMISSION_MODE" });
			expect(next.permissionMode).toBe("default");
		});
	});

	describe("SET_PENDING_PERMISSION", () => {
		it("sets pending permission request for a session", () => {
			const request = {
				request_id: "req-1",
				tool_name: "Edit",
				input: { file_path: "/src/index.ts" },
				tool_use_id: "toolu_001",
				title: "Edit file",
			};
			const next = reducer(INITIAL_STATE, {
				type: "SET_PENDING_PERMISSION",
				sessionId: "s1",
				request,
			});
			expect(next.pendingPermissions.s1).toBe(request);
		});

		it("clears pending permission with null", () => {
			const request = {
				request_id: "req-1",
				tool_name: "Edit",
				input: {},
				tool_use_id: "toolu_001",
			};
			const state: AgentChatState = {
				...INITIAL_STATE,
				pendingPermissions: { s1: request },
			};
			const next = reducer(state, {
				type: "SET_PENDING_PERMISSION",
				sessionId: "s1",
				request: null,
			});
			expect(next.pendingPermissions.s1).toBeUndefined();
		});

		it("stores permissions for multiple sessions independently", () => {
			const req1 = {
				request_id: "req-1",
				tool_name: "Edit",
				input: {},
				tool_use_id: "toolu_001",
			};
			const req2 = {
				request_id: "req-2",
				tool_name: "Bash",
				input: {},
				tool_use_id: "toolu_002",
			};
			const step1 = reducer(INITIAL_STATE, {
				type: "SET_PENDING_PERMISSION",
				sessionId: "s1",
				request: req1,
			});
			const step2 = reducer(step1, {
				type: "SET_PENDING_PERMISSION",
				sessionId: "s2",
				request: req2,
			});
			expect(step2.pendingPermissions.s1).toBe(req1);
			expect(step2.pendingPermissions.s2).toBe(req2);
		});
	});

	describe("SET_AGENT_SESSION_ID", () => {
		it("sets agent session id on active session", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession(),
			};
			const next = reducer(state, {
				type: "SET_AGENT_SESSION_ID",
				agentSessionId: "sess-xyz",
			});
			expect(next.activeSession?.agentSessionId).toBe("sess-xyz");
		});

		it("clears agent session id with null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ agentSessionId: "sess-xyz" }),
			};
			const next = reducer(state, {
				type: "SET_AGENT_SESSION_ID",
				agentSessionId: null,
			});
			expect(next.activeSession?.agentSessionId).toBeNull();
		});

		it("does nothing when no active session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_AGENT_SESSION_ID",
				agentSessionId: "sess-xyz",
			});
			expect(next).toBe(INITIAL_STATE);
		});
	});
});
