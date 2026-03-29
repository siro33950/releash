import { describe, expect, it, vi } from "vitest";
import type { LegacyChatMessage } from "@/types/session";
import { legacyToParts } from "./useSessionStore";

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
