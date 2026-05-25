import { describe, expect, it } from "vitest";
import type { MessagePart } from "./session";
import { getTextContent, normalizePermissionMode } from "./session";

describe("getTextContent", () => {
	it("joins text parts into a single string", () => {
		const parts: MessagePart[] = [
			{ type: "text", content: "Hello " },
			{ type: "text", content: "world" },
		];
		expect(getTextContent(parts)).toBe("Hello world");
	});

	it("extracts only text parts from mixed parts", () => {
		const parts: MessagePart[] = [
			{ type: "thinking", content: "hmm" },
			{ type: "text", content: "answer" },
			{ type: "error", content: "oops" },
		];
		expect(getTextContent(parts)).toBe("answer");
	});

	it("returns empty string when no text parts exist", () => {
		const parts: MessagePart[] = [
			{ type: "thinking", content: "hmm" },
			{ type: "error", content: "oops" },
		];
		expect(getTextContent(parts)).toBe("");
	});

	it("returns empty string for empty array", () => {
		expect(getTextContent([])).toBe("");
	});

	it("handles tool_result with toolUseId in mixed parts", () => {
		const parts: MessagePart[] = [
			{ type: "text", content: "before" },
			{
				type: "tool_result",
				content: "result",
				isError: false,
				toolUseId: "toolu_123",
			},
			{ type: "text", content: "after" },
		];
		expect(getTextContent(parts)).toBe("beforeafter");
	});
});

describe("normalizePermissionMode", () => {
	it("normalizes canonical, display, and legacy permission modes", () => {
		expect(normalizePermissionMode("ask")).toBe("ask");
		expect(normalizePermissionMode("Ask")).toBe("ask");
		expect(normalizePermissionMode(" ask ")).toBe("ask");
		expect(normalizePermissionMode("edit")).toBe("edit");
		expect(normalizePermissionMode("EDIT")).toBe("edit");
		expect(normalizePermissionMode("full")).toBe("full");
		expect(normalizePermissionMode(" Full ")).toBe("full");
		expect(normalizePermissionMode("readonly")).toBe("ask");
		expect(normalizePermissionMode("READONLY")).toBe("ask");
	});

	it("keeps the existing fallback for unknown values", () => {
		expect(normalizePermissionMode("unknown")).toBe("edit");
		expect(normalizePermissionMode(null)).toBe("edit");
		expect(normalizePermissionMode(undefined)).toBe("edit");
	});
});
