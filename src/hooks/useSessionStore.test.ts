import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	getSession,
	getSessionPage,
	planAgentChatEviction,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
} from "./useSessionStore";

function makeRawSendMessageResponse() {
	return {
		session: {
			id: "s1",
			worktreePath: "/repo",
			messages: [],
			state: "active",
			createdAt: 1,
			updatedAt: 1,
			permissionMode: "edit",
		},
		humanMessage: {
			id: "h1",
			role: "human",
			content: "approve",
			timestamp: 2,
		},
		agentMessage: null,
		sessions: [],
	};
}

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

describe("session paging", () => {
	beforeEach(() => {
		vi.mocked(invoke).mockReset();
	});

	it("getSession uses messages and initial page metadata returned by get_session", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			id: "s1",
			worktreePath: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					content: "hello",
					timestamp: 1001,
				},
			],
			state: "active",
			createdAt: 1000,
			updatedAt: 1000,
			permissionMode: "edit",
			selectedModel: "claude:sonnet",
			turnPhase: "idle",
			availableModels: [],
			initialPage: {
				nextCursor: "1",
				hasMore: true,
				totalCount: 10,
			},
			latestTokenUsage: { inputTokens: 1, outputTokens: 2 },
		});

		const response = await getSession("s1");

		expect(invoke).toHaveBeenCalledTimes(1);
		expect(invoke).toHaveBeenCalledWith("get_session", {
			sessionId: "s1",
		});
		expect(response?.session.messages).toEqual([
			{
				id: "m1",
				role: "human",
				parts: [{ type: "text", content: "hello" }],
				timestamp: 1001,
				mentions: undefined,
			},
		]);
		expect(response?.initialPage).toEqual({
			nextCursor: "1",
			hasMore: true,
			totalCount: 10,
		});
		expect(response?.latestTokenUsage).toEqual({
			inputTokens: 1,
			outputTokens: 2,
		});
	});

	it("getSessionPage forwards cursor and limit", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			messages: [
				{
					id: "m2",
					role: "human",
					content: "older",
					timestamp: 1000,
				},
			],
			messageMetadata: [{ messageId: "m2", tokenMeta: { input: 1 } }],
			nextCursor: null,
			hasMore: false,
			totalCount: 1,
			latestTokenUsage: null,
		});

		const page = await getSessionPage("s1", "7", 25);

		expect(invoke).toHaveBeenCalledWith("get_session_page", {
			sessionId: "s1",
			cursor: "7",
			limit: 25,
		});
		expect(page).toEqual({
			messages: [
				{
					id: "m2",
					role: "human",
					parts: [{ type: "text", content: "older" }],
					timestamp: 1000,
					mentions: undefined,
				},
			],
			messageMetadata: [{ messageId: "m2", tokenMeta: { input: 1 } }],
			nextCursor: null,
			hasMore: false,
			totalCount: 1,
			latestTokenUsage: null,
		});
	});

	it("planAgentChatEviction forwards request and returns the plan unchanged", async () => {
		const request = {
			active: {
				sessionId: "s1",
				messageCount: 250,
				oldestVisibleIndex: 50,
				loadedPages: [
					{ requestCursor: null, count: 50 },
					{ requestCursor: "201", count: 50 },
				],
				turnPhase: "idle" as const,
			},
			sessions: [
				{
					sessionId: "s2",
					messageCount: 50,
					evictionRank: 1,
					protected: false,
					loading: false,
				},
			],
		};
		const plan = {
			active: {
				sessionId: "s1",
				direction: "older" as const,
				count: 50,
				nextCursor: "201",
				hasMore: true,
				loadedPages: [{ requestCursor: null, count: 50 }],
			},
			evictSessionIds: ["s2"],
		};
		vi.mocked(invoke).mockResolvedValueOnce(plan);

		const response = await planAgentChatEviction(request);

		expect(invoke).toHaveBeenCalledWith("plan_agent_chat_eviction", {
			request,
		});
		expect(response).toBe(plan);
	});

	it("sendWorkflowApprovalChatMessage omits client timing args", async () => {
		vi.mocked(invoke).mockResolvedValueOnce(makeRawSendMessageResponse());

		await sendWorkflowApprovalChatMessage("run-1", "approve", "edit", false);

		expect(invoke).toHaveBeenCalledWith("send_workflow_approval_chat_message", {
			runId: "run-1",
			content: "approve",
			permissionMode: "edit",
			planMode: false,
			images: undefined,
			mentions: undefined,
		});
	});

	it("sendAgentMessage omits client timing args", async () => {
		vi.mocked(invoke).mockResolvedValueOnce(makeRawSendMessageResponse());

		await sendAgentMessage(
			"s1",
			"/repo",
			"hello",
			"edit",
			false,
			"claude",
			[],
			[],
			undefined,
			"claude-sonnet-4-6",
		);

		expect(invoke).toHaveBeenCalledWith("send_agent_message", {
			chatSessionId: "s1",
			worktreePath: "/repo",
			content: "hello",
			permissionMode: "edit",
			planMode: false,
			backendId: "claude",
			modelId: "claude-sonnet-4-6",
			images: undefined,
			mentions: undefined,
		});
	});
});
