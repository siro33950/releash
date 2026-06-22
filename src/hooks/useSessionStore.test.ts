import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LegacyChatMessage } from "@/types/session";
import {
	getSession,
	getSessionPage,
	legacyToParts,
	planAgentChatEviction,
} from "./useSessionStore";

function makeLegacyMsg(
	overrides?: Partial<LegacyChatMessage>,
): LegacyChatMessage {
	return {
		id: "m1",
		role: "agent",
		content: "",
		timestamp: 1000,
		...overrides,
	};
}

describe("legacyToParts", () => {
	it("thinking のみ → [{ type: 'thinking', content }]", () => {
		const msg = makeLegacyMsg({ thinking: "reasoning..." });
		expect(legacyToParts(msg)).toEqual([
			{ type: "thinking", content: "reasoning..." },
		]);
	});

	it("content のみ → [{ type: 'text', content }]", () => {
		const msg = makeLegacyMsg({ content: "hello" });
		expect(legacyToParts(msg)).toEqual([{ type: "text", content: "hello" }]);
	});

	it("activities に tool_use → 対応する MessagePart に変換", () => {
		const msg = makeLegacyMsg({
			activities: [
				{ type: "tool_use", tool: "Read", input: { path: "/a" }, id: "tu1" },
			],
		});
		expect(legacyToParts(msg)).toEqual([
			{ type: "tool_use", tool: "Read", input: { path: "/a" }, id: "tu1" },
		]);
	});

	it("activities に tool_result → 対応する MessagePart に変換", () => {
		const msg = makeLegacyMsg({
			activities: [
				{ type: "tool_result", content: "file content", isError: false },
			],
		});
		expect(legacyToParts(msg)).toEqual([
			{ type: "tool_result", content: "file content", isError: false },
		]);
	});

	it("activities に permission_result (allowed) → { type: 'permission', status: 'allowed' }", () => {
		const msg = makeLegacyMsg({
			activities: [
				{
					type: "permission_result",
					toolName: "Write",
					status: "allowed",
					summary: "ok",
				},
			],
		});
		const parts = legacyToParts(msg);
		expect(parts).toHaveLength(1);
		expect(parts[0]).toMatchObject({
			type: "permission",
			status: "allowed",
			request: { tool_name: "Write" },
		});
	});

	it("activities に permission_result (denied) → { type: 'permission', status: 'denied' }", () => {
		const msg = makeLegacyMsg({
			activities: [
				{
					type: "permission_result",
					toolName: "Bash",
					status: "denied",
					summary: "no",
				},
			],
		});
		const parts = legacyToParts(msg);
		expect(parts).toHaveLength(1);
		expect(parts[0]).toMatchObject({
			type: "permission",
			status: "denied",
			request: { tool_name: "Bash" },
		});
	});

	it("全フィールド → thinking, activities, content の順序で配列に含まれる", () => {
		const msg = makeLegacyMsg({
			thinking: "let me think",
			activities: [
				{ type: "tool_use", tool: "Read", input: {}, id: "tu1" },
				{ type: "tool_result", content: "done", isError: false },
			],
			content: "result",
		});
		const parts = legacyToParts(msg);
		expect(parts).toEqual([
			{ type: "thinking", content: "let me think" },
			{ type: "tool_use", tool: "Read", input: {}, id: "tu1" },
			{ type: "tool_result", content: "done", isError: false },
			{ type: "text", content: "result" },
		]);
	});

	it("全フィールド空/undefined → 空配列", () => {
		const msg = makeLegacyMsg({
			content: "",
			thinking: undefined,
			activities: undefined,
		});
		expect(legacyToParts(msg)).toEqual([]);
	});
});

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
});
