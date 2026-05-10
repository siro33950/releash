import { describe, expect, it } from "vitest";
import type { BackendInfo, ChatMessage, ChatSession } from "@/types/session";
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
		permissionMode: "acceptEdits" as const,
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
			turnPhases: {},
			error: null,
			permissionMode: "acceptEdits" as const,
			pendingPermissions: {},
			availableModels: [],
			sessionModels: {},
			backends: [],
			selectedBackendId: null,
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
					permissionMode: "acceptEdits" as const,
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
						permissionMode: "acceptEdits" as const,
					},
					{
						id: "s2",
						worktreePath: "/repo",
						state: "idle" as const,
						createdAt: 900,
						updatedAt: 900,
						firstMessage: "second",
						messageCount: 1,
						permissionMode: "acceptEdits" as const,
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
					permissionMode: "acceptEdits" as const,
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
					permissionMode: "acceptEdits" as const,
				},
				{
					id: "s3",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1100,
					updatedAt: 1100,
					firstMessage: "third",
					messageCount: 1,
					permissionMode: "acceptEdits" as const,
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
					permissionMode: "acceptEdits" as const,
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

	describe("SET_TURN_PHASE", () => {
		it("sets turn phase for a session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_TURN_PHASE",
				sessionId: "s1",
				turnPhase: "streaming",
			});
			expect(next.turnPhases).toEqual({ s1: "streaming" });
		});

		it("overwrites existing turn phase", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				turnPhases: { s1: "streaming" },
			};
			const next = reducer(state, {
				type: "SET_TURN_PHASE",
				sessionId: "s1",
				turnPhase: "idle",
			});
			expect(next.turnPhases).toEqual({ s1: "idle" });
		});

		it("supports multiple concurrent sessions", () => {
			const step1 = reducer(INITIAL_STATE, {
				type: "SET_TURN_PHASE",
				sessionId: "s1",
				turnPhase: "streaming",
			});
			const step2 = reducer(step1, {
				type: "SET_TURN_PHASE",
				sessionId: "s2",
				turnPhase: "waiting_permission",
			});
			expect(step2.turnPhases).toEqual({
				s1: "streaming",
				s2: "waiting_permission",
			});
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

		it("updates existing compaction notification in-place", () => {
			const existing = [
				{
					type: "system_notification" as const,
					notificationType: "compaction" as const,
					status: "in_progress" as const,
					label: "Compacting conversation...",
				},
			];
			const delta = [
				{
					type: "system_notification" as const,
					notificationType: "compaction" as const,
					status: "completed" as const,
					label: "Conversation compacted",
					detail: "trigger=auto, 50000 tokens",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(1);
			expect(result[0]).toEqual({
				type: "system_notification",
				notificationType: "compaction",
				status: "completed",
				label: "Conversation compacted",
				detail: "trigger=auto, 50000 tokens",
			});
		});

		it("updates existing hook notification by hookId", () => {
			const existing = [
				{
					type: "system_notification" as const,
					notificationType: "hook" as const,
					status: "in_progress" as const,
					label: "SessionEnd (StopSession)",
					hookId: "hook-001",
				},
			];
			const delta = [
				{
					type: "system_notification" as const,
					notificationType: "hook" as const,
					status: "completed" as const,
					label: "SessionEnd (StopSession)",
					detail: "outcome=success, exit_code=0",
					hookId: "hook-001",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(1);
			expect(result[0]).toEqual(
				expect.objectContaining({
					status: "completed",
					detail: "outcome=success, exit_code=0",
				}),
			);
		});

		it("appends new system_notification when no match found", () => {
			const existing = [{ type: "text" as const, content: "hello" }];
			const delta = [
				{
					type: "system_notification" as const,
					notificationType: "files_persisted" as const,
					status: "completed" as const,
					label: "Files persisted",
					detail: "CLAUDE.md",
				},
			];
			const result = mergeDeltaParts(existing, delta);
			expect(result).toHaveLength(2);
			expect(result[1].type).toBe("system_notification");
		});
	});

	describe("SET_AVAILABLE_MODELS", () => {
		it("stores available models globally", () => {
			const models = [
				{ value: "claude-4", displayName: "Claude 4" },
				{ value: "claude-3.5", displayName: "Claude 3.5" },
			];
			const next = reducer(INITIAL_STATE, {
				type: "SET_AVAILABLE_MODELS",
				models,
			});
			expect(next.availableModels).toBe(models);
		});
	});

	describe("SET_SESSION_MODEL", () => {
		it("sets selected model for a session", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_SESSION_MODEL",
				sessionId: "s1",
				modelId: "claude-4",
			});
			expect(next.sessionModels.s1).toBe("claude-4");
		});

		it("clears model selection with null (SDK default)", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionModels: { s1: "claude-4" },
			};
			const next = reducer(state, {
				type: "SET_SESSION_MODEL",
				sessionId: "s1",
				modelId: null,
			});
			expect(next.sessionModels.s1).toBeNull();
		});

		it("stores models for multiple sessions independently", () => {
			let state = reducer(INITIAL_STATE, {
				type: "SET_SESSION_MODEL",
				sessionId: "s1",
				modelId: "claude-4",
			});
			state = reducer(state, {
				type: "SET_SESSION_MODEL",
				sessionId: "s2",
				modelId: "claude-3.5",
			});
			expect(state.sessionModels.s1).toBe("claude-4");
			expect(state.sessionModels.s2).toBe("claude-3.5");
		});
	});

	describe("CLEANUP_SESSION", () => {
		it("removes session entries from all Record fields", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				turnPhases: { s1: "streaming", s2: "idle" },
				pendingPermissions: {
					s1: {
						request_id: "req-1",
						tool_name: "Edit",
						input: {},
						tool_use_id: "toolu_001",
					},
				},
				sessionModels: { s1: "claude-4", s2: null },
			};
			const next = reducer(state, {
				type: "CLEANUP_SESSION",
				sessionId: "s1",
			});
			expect(next.turnPhases).toEqual({ s2: "idle" });
			expect(next.pendingPermissions).toEqual({});
			expect(next.sessionModels).toEqual({ s2: null });
		});

		it("is a no-op when session ID does not exist in any Record", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				turnPhases: { s1: "idle" },
				sessionModels: { s1: "claude-4" },
			};
			const next = reducer(state, {
				type: "CLEANUP_SESSION",
				sessionId: "nonexistent",
			});
			expect(next.turnPhases).toEqual({ s1: "idle" });
			expect(next.sessionModels).toEqual({ s1: "claude-4" });
		});
	});

	describe("SET_BACKENDS", () => {
		const backend1: BackendInfo = {
			id: "b1",
			name: "Backend 1",
			available: true,
		};
		const backend2: BackendInfo = {
			id: "b2",
			name: "Backend 2",
			available: true,
		};

		it("sets selectedBackendId to defaultId when selectedBackendId is null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: null,
			};
			const next = reducer(state, {
				type: "SET_BACKENDS",
				backends: [backend1, backend2],
				defaultId: "b2",
			});
			expect(next.backends).toEqual([backend1, backend2]);
			expect(next.selectedBackendId).toBe("b2");
		});

		it("selects the first backend when selectedBackendId is null and defaultId is null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: null,
			};
			const next = reducer(state, {
				type: "SET_BACKENDS",
				backends: [backend1, backend2],
				defaultId: null,
			});
			expect(next.backends).toEqual([backend1, backend2]);
			expect(next.selectedBackendId).toBe("b1");
		});

		it("preserves existing selectedBackendId when already set", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: "b2",
			};
			const next = reducer(state, {
				type: "SET_BACKENDS",
				backends: [backend1, backend2],
				defaultId: "b1",
			});
			expect(next.backends).toEqual([backend1, backend2]);
			expect(next.selectedBackendId).toBe("b2");
		});

		it("sets selectedBackendId to null when backends are empty and defaultId is null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: null,
			};
			const next = reducer(state, {
				type: "SET_BACKENDS",
				backends: [],
				defaultId: null,
			});
			expect(next.backends).toEqual([]);
			expect(next.selectedBackendId).toBeNull();
		});
	});

	describe("SET_SELECTED_BACKEND", () => {
		it("updates selectedBackendId with backendId", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_SELECTED_BACKEND",
				backendId: "b1",
			});
			expect(next.selectedBackendId).toBe("b1");
		});

		it("clears selectedBackendId with null", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: "b1",
			};
			const next = reducer(state, {
				type: "SET_SELECTED_BACKEND",
				backendId: null,
			});
			expect(next.selectedBackendId).toBeNull();
		});
	});
});
