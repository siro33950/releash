import { describe, expect, it } from "vitest";
import { formatImplementPrompt } from "./formatImplementPrompt";

describe("formatImplementPrompt", () => {
	it("should include the thread ID in the prompt", () => {
		const prompt = formatImplementPrompt("t-42");
		expect(prompt).toContain('thread "t-42"');
	});

	it("should reference the get_thread MCP tool", () => {
		const prompt = formatImplementPrompt("t-1");
		expect(prompt).toContain("get_thread");
		expect(prompt).toContain('thread_id="t-1"');
	});

	it("should reference the resolve_thread tool", () => {
		const prompt = formatImplementPrompt("t-1");
		expect(prompt).toContain("resolve_thread");
	});
});
