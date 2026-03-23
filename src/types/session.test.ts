import { describe, expect, it } from "vitest";
import type { MessagePart } from "./session";
import { getTextContent } from "./session";

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
