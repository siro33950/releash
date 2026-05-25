import { describe, expect, it } from "vitest";
import type { BackendInfo, ChatMessage, ChatSession } from "@/types/session";
import type { AgentChatState } from "./agentChatReducer";
import {
	INITIAL_STATE,
	reducer,
	selectActiveSession,
	selectSessionFromState,
} from "./agentChatReducer";

function makeSession(overrides?: Partial<ChatSession>): ChatSession {
	return {
		id: "s1",
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode: "edit" as const,
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
			sessionsById: {},
			activeSessionId: null,
			turnPhases: {},
			error: null,
			permissionMode: "edit" as const,
			pendingPermissions: {},
			availableModels: [],
			availableModelsByBackend: {},
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
					permissionMode: "edit" as const,
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
						permissionMode: "edit" as const,
					},
					{
						id: "s2",
						worktreePath: "/repo",
						state: "idle" as const,
						createdAt: 900,
						updatedAt: 900,
						firstMessage: "second",
						messageCount: 1,
						permissionMode: "edit" as const,
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
					permissionMode: "edit" as const,
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
					permissionMode: "edit" as const,
				},
				{
					id: "s3",
					worktreePath: "/repo",
					state: "idle" as const,
					createdAt: 1100,
					updatedAt: 1100,
					firstMessage: "third",
					messageCount: 1,
					permissionMode: "edit" as const,
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
					permissionMode: "edit" as const,
				},
			];
			const next = reducer(INITIAL_STATE, {
				type: "SET_CLOSED_SESSIONS",
				sessions,
			});
			expect(next.closedSessions).toBe(sessions);
		});
	});

	describe("UPSERT_SESSION + SET_ACTIVE_SESSION_ID", () => {
		it("UPSERT_SESSION stores session in sessionsById", () => {
			const session = makeSession();
			const next = reducer(INITIAL_STATE, {
				type: "UPSERT_SESSION",
				session,
			});
			expect(next.sessionsById[session.id]).toBe(session);
			expect(next.error).toBeNull();
		});

		it("UPSERT_SESSION clears error", () => {
			const stateWithError: AgentChatState = {
				...INITIAL_STATE,
				error: "some error",
			};
			const session = makeSession();
			const next = reducer(stateWithError, {
				type: "UPSERT_SESSION",
				session,
			});
			expect(next.error).toBeNull();
		});

		it("SET_ACTIVE_SESSION_ID resolves active session from sessionsById", () => {
			const session = makeSession();
			const upserted = reducer(INITIAL_STATE, {
				type: "UPSERT_SESSION",
				session,
			});
			const next = reducer(upserted, {
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: session.id,
			});
			expect(next.activeSessionId).toBe(session.id);
			expect(selectActiveSession(next)).toBe(session);
		});

		it("SET_ACTIVE_SESSION_ID null clears the active selection", () => {
			const session = makeSession();
			const upserted = reducer(INITIAL_STATE, {
				type: "UPSERT_SESSION",
				session,
			});
			const withActive = reducer(upserted, {
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: session.id,
			});
			const next = reducer(withActive, {
				type: "SET_ACTIVE_SESSION_ID",
				sessionId: null,
			});
			expect(next.activeSessionId).toBeNull();
			expect(selectActiveSession(next)).toBeNull();
		});
	});

	describe("ADD_MESSAGE", () => {
		it("appends message to session in sessionsById when sessionId matches", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { s1: makeSession({ id: "s1" }) },
			};
			const msg = makeMessage();
			const next = reducer(state, {
				type: "ADD_MESSAGE",
				sessionId: "s1",
				message: msg,
			});
			expect(next.sessionsById.s1.messages).toHaveLength(1);
			expect(next.sessionsById.s1.messages[0]).toBe(msg);
		});

		it("does nothing when session is not in sessionsById", () => {
			const msg = makeMessage();
			const next = reducer(INITIAL_STATE, {
				type: "ADD_MESSAGE",
				sessionId: "s1",
				message: msg,
			});
			expect(next).toBe(INITIAL_STATE);
		});

		it("appends to step session in sessionsById when its id matches", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { "step-1": makeSession({ id: "step-1" }) },
			};
			const msg = makeMessage();
			const next = reducer(state, {
				type: "ADD_MESSAGE",
				sessionId: "step-1",
				message: msg,
			});
			expect(next.sessionsById["step-1"].messages).toHaveLength(1);
			expect(next.sessionsById["step-1"].messages[0]).toBe(msg);
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
		it("updates session state in sessionsById when sessionId matches", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { s1: makeSession({ id: "s1", state: "active" }) },
			};
			const next = reducer(state, {
				type: "UPDATE_SESSION_STATE",
				sessionId: "s1",
				state: "done",
			});
			expect(next.sessionsById.s1.state).toBe("done");
		});

		it("does nothing when no session matches", () => {
			const next = reducer(INITIAL_STATE, {
				type: "UPDATE_SESSION_STATE",
				sessionId: "s1",
				state: "done",
			});
			expect(next).toBe(INITIAL_STATE);
		});

		it("updates step session state in sessionsById when its id matches", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: {
					"step-1": makeSession({ id: "step-1", state: "active" }),
				},
			};
			const next = reducer(state, {
				type: "UPDATE_SESSION_STATE",
				sessionId: "step-1",
				state: "done",
			});
			expect(next.sessionsById["step-1"].state).toBe("done");
		});
	});

	describe("SET_PERMISSION_MODE", () => {
		it("updates permissionMode to ask", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_PERMISSION_MODE",
				mode: "ask",
			});
			expect(next.permissionMode).toBe("ask");
		});

		it("switches from ask to full", () => {
			const state: AgentChatState = {
				...INITIAL_STATE,
				permissionMode: "ask",
			};
			const next = reducer(state, {
				type: "SET_PERMISSION_MODE",
				mode: "full",
			});
			expect(next.permissionMode).toBe("full");
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
		it("replaces existing parts with the cumulative payload in sessionsById", () => {
			// Rust sends the full cumulative `streaming_parts` on every flush, so the
			// reducer replaces the message's parts wholesale. A redelivery (same or
			// extended cumulative payload) must converge without double-application.
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "old" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { s1: makeSession({ id: "s1", messages: [msg] }) },
			};
			const cumulativeParts = [
				{ type: "text" as const, content: "old updated" },
				{ type: "thinking" as const, content: "reasoning" },
			];
			const next = reducer(state, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: cumulativeParts,
			});
			expect(next.sessionsById.s1.messages[0].parts).toEqual(cumulativeParts);
		});

		it("converges on re-delivery of the same cumulative payload", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "old" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { s1: makeSession({ id: "s1", messages: [msg] }) },
			};
			const cumulative = [{ type: "text" as const, content: "old updated" }];
			const once = reducer(state, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: cumulative,
			});
			const twice = reducer(once, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: cumulative,
			});
			expect(twice.sessionsById.s1.messages[0].parts).toEqual(cumulative);
		});

		it("does nothing when target session is missing from sessionsById", () => {
			const next = reducer(INITIAL_STATE, {
				type: "SET_STREAMING_MESSAGE",
				sessionId: "s1",
				messageId: "m1",
				parts: [{ type: "text", content: "hello" }],
			});
			expect(next).toBe(INITIAL_STATE);
		});

		it("does nothing when sessionId does not match any session in store", () => {
			const msg = makeMessage({
				id: "m1",
				role: "agent",
				parts: [{ type: "text", content: "original" }],
			});
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { s1: makeSession({ id: "s1", messages: [msg] }) },
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
				sessionsById: { s1: makeSession({ id: "s1", messages: [msg] }) },
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

	describe("selectors", () => {
		it("selectSessionFromState returns null when sessionsById has no entry", () => {
			expect(selectSessionFromState(INITIAL_STATE, "missing")).toBeNull();
			expect(selectSessionFromState(INITIAL_STATE, null)).toBeNull();
		});

		it("selectSessionFromState returns the session from sessionsById", () => {
			const session = makeSession();
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { [session.id]: session },
			};
			expect(selectSessionFromState(state, session.id)).toBe(session);
		});

		it("selectActiveSession resolves via activeSessionId", () => {
			const session = makeSession({ id: "active" });
			const state: AgentChatState = {
				...INITIAL_STATE,
				sessionsById: { [session.id]: session },
				activeSessionId: session.id,
			};
			expect(selectActiveSession(state)).toBe(session);
		});
	});

	describe("SET_AVAILABLE_MODELS", () => {
		it("stores available models globally", () => {
			const models = [{ value: "claude-4" }, { value: "claude-3.5" }];
			const next = reducer(INITIAL_STATE, {
				type: "SET_AVAILABLE_MODELS",
				models,
			});
			expect(next.availableModels).toBe(models);
		});

		it("stores available models by backend when backendId is provided", () => {
			const models = [{ value: "claude-4" }];
			const next = reducer(INITIAL_STATE, {
				type: "SET_AVAILABLE_MODELS",
				models,
				backendId: "claude",
			});
			expect(next.availableModelsByBackend.claude).toBe(models);
		});
	});

	describe("SET_BACKEND_MODELS", () => {
		it("stores backend models and updates visible models for current backend", () => {
			const models = [{ value: "gpt-5.5" }];
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: "codex",
			};
			const next = reducer(state, {
				type: "SET_BACKEND_MODELS",
				backendId: "codex",
				models,
			});
			expect(next.availableModels).toBe(models);
			expect(next.availableModelsByBackend.codex).toBe(models);
		});

		it("stores backend models without changing visible models for another backend", () => {
			const visible = [{ value: "claude-4" }];
			const codexModels = [{ value: "gpt-5.5" }];
			const state: AgentChatState = {
				...INITIAL_STATE,
				selectedBackendId: "claude",
				availableModels: visible,
			};
			const next = reducer(state, {
				type: "SET_BACKEND_MODELS",
				backendId: "codex",
				models: codexModels,
			});
			expect(next.availableModels).toBe(visible);
			expect(next.availableModelsByBackend.codex).toBe(codexModels);
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
			availableModels: [{ value: "b1-model" }],
		};
		const backend2: BackendInfo = {
			id: "b2",
			name: "Backend 2",
			available: true,
			availableModels: [{ value: "b2-model" }],
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
			expect(next.availableModels).toEqual([{ value: "b2-model" }]);
			expect(next.availableModelsByBackend).toEqual({
				b1: [{ value: "b1-model" }],
				b2: [{ value: "b2-model" }],
			});
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
			const state: AgentChatState = {
				...INITIAL_STATE,
				availableModelsByBackend: { b1: [{ value: "model-1" }] },
			};
			const next = reducer(state, {
				type: "SET_SELECTED_BACKEND",
				backendId: "b1",
			});
			expect(next.selectedBackendId).toBe("b1");
			expect(next.availableModels).toEqual([{ value: "model-1" }]);
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
