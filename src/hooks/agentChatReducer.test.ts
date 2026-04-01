import { describe, expect, it } from "vitest";
import type { ChatMessage, ChatSession } from "@/types/session";
import type { AgentChatState } from "./agentChatReducer";
import { INITIAL_STATE, mergeDeltaParts, reducer } from "./agentChatReducer";

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
			sessionFinalStates: {},
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

		it("restores bypassPermissions after Agent changes to plan then default", () => {
			// Scenario: Bypass → Agent sets Plan → Agent restores default → Bypass
			let state: AgentChatState = {
				...INITIAL_STATE,
			};
			// User selects bypassPermissions
			state = reducer(state, {
				type: "SET_USER_PERMISSION_MODE",
				mode: "bypassPermissions",
			});
			expect(state.permissionMode).toBe("bypassPermissions");
			expect(state.userPermissionMode).toBe("bypassPermissions");

			// Agent changes to plan
			state = reducer(state, {
				type: "SET_PERMISSION_MODE",
				mode: "plan",
			});
			expect(state.permissionMode).toBe("plan");
			expect(state.userPermissionMode).toBe("bypassPermissions");

			// Agent restores default → should restore to bypassPermissions
			state = reducer(state, { type: "RESTORE_USER_PERMISSION_MODE" });
			expect(state.permissionMode).toBe("bypassPermissions");
			expect(state.userPermissionMode).toBe("bypassPermissions");
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

	describe("SET_STREAMING_MESSAGE", () => {
		it("appends delta parts to existing parts in active session", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "old" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ id: "s1", messages: [msg] }),
			};
			const deltaParts = [
				{ type: "text" as const, content: " updated" },
				{ type: "thinking" as const, content: "reasoning" },
			];
			const next = reducer(state, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: deltaParts,
			});
			expect(next.activeSession?.messages[0].parts).toEqual([
				{ type: "text", content: "old updated" },
				{ type: "thinking", content: "reasoning" },
			]);
		});

		it("does nothing when activeSession is null", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: [{ type: "text", content: "hello" }],
			});
			expect(next).toBe(INITIAL_STATE);
		});

		it("does nothing when sessionId does not match activeSession", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "original" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ id: "s1", messages: [msg] }),
			};
			const next = reducer(state, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s2",
				messageId: "m1",
				parts: [{ type: "text", content: "should not apply" }],
			});
			expect(next).toBe(state);
		});

		it("does nothing when messageId is not found", () => {
			const msg = makeMessage({ id: "m1", role: "agent" });
			const state: AgentChatState = {
				...INITIAL_STATE,
				activeSession: makeSession({ id: "s1", messages: [msg] }),
			};
			const next = reducer(state, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "nonexistent",
				parts: [{ type: "text", content: "hello" }],
			});
			expect(next).toBe(state);
		});
	});

	describe("mergeDeltaParts", () => {
		it("merges consecutive text delta into last text part", () => {
			const existing = [{ type: "text" as const, content: "Hello" }];
			const delta = [{ type: "text" as const, content: " World" }];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toEqual([{ type: "text", content: "Hello World" }]);
		});

		it("appends text delta when last part is different type", () => {
			const existing = [{ type: "thinking" as const, content: "thinking..." }];
			const delta = [{ type: "text" as const, content: "answer" }];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(2);
			expect(result[1]).toEqual({ type: "text", content: "answer" });
		});

		it("adds new permission part", () => {
			const existing: import("@/types/session").MessagePart[] = [];
			const delta: import("@/types/session").MessagePart[] = [
				{
					type: "permission",
					request: {
						request_id: "req-1",
						tool_name: "ExitPlanMode",
						input: {},
						tool_use_id: "toolu_001",
					},
					status: "pending",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(1);
			expect(result[0]).toEqual({
				type: "permission",
				request: {
					request_id: "req-1",
					tool_name: "ExitPlanMode",
					input: {},
					tool_use_id: "toolu_001",
				},
				status: "pending",
			});
		});

		it("updates existing permission part by request_id", () => {
			const existing: import("@/types/session").MessagePart[] = [
				{
					type: "permission",
					request: {
						request_id: "req-1",
						tool_name: "ExitPlanMode",
						input: {},
						tool_use_id: "toolu_001",
					},
					status: "pending",
				},
			];
			const delta: import("@/types/session").MessagePart[] = [
				{
					type: "permission",
					request: {
						request_id: "req-1",
						tool_name: "ExitPlanMode",
						input: {},
						tool_use_id: "toolu_001",
					},
					status: "allowed",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(1);
			expect(result[0]).toEqual({
				type: "permission",
				request: {
					request_id: "req-1",
					tool_name: "ExitPlanMode",
					input: {},
					tool_use_id: "toolu_001",
				},
				status: "allowed",
			});
		});

		it("appends tool_use parts", () => {
			const existing = [{ type: "text" as const, content: "hello" }];
			const delta: import("@/types/session").MessagePart[] = [
				{
					type: "tool_use",
					tool: "Edit",
					input: { file_path: "/src/main.rs" },
					id: "toolu_001",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(2);
			expect(result[1].type).toBe("tool_use");
		});

		it("returns existing when delta is empty", () => {
			const existing = [{ type: "text" as const, content: "hello" }];
			const result = mergeDeltaParts(existing, []);
			expect(result).toBe(existing);
		});

		it("does not merge text with different parentToolUseId", () => {
			const existing = [{ type: "text" as const, content: "main" }];
			const delta = [
				{ type: "text" as const, content: "sub", parentToolUseId: "parent1" },
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(2);
		});
	});
});
