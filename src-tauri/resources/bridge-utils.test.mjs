import { describe, expect, it } from "vitest";
import { buildSystemPromptOption } from "./bridge-utils.mjs";

describe("buildSystemPromptOption", () => {
	it("transforms systemPrompt string into preset append format", () => {
		const result = buildSystemPromptOption("You are a coder.");
		expect(result).toEqual({
			systemPrompt: {
				type: "preset",
				preset: "claude_code",
				append: "You are a coder.",
			},
		});
	});

	it("returns empty object when systemPrompt is undefined", () => {
		expect(buildSystemPromptOption(undefined)).toEqual({});
	});

	it("returns empty object when systemPrompt is null", () => {
		expect(buildSystemPromptOption(null)).toEqual({});
	});

	it("returns empty object when systemPrompt is empty string", () => {
		expect(buildSystemPromptOption("")).toEqual({});
	});

	it("preserves multiline persona content", () => {
		const persona = "You are a planner.\n\nYour job is to create plans.";
		const result = buildSystemPromptOption(persona);
		expect(result.systemPrompt.append).toBe(persona);
		expect(result.systemPrompt.type).toBe("preset");
		expect(result.systemPrompt.preset).toBe("claude_code");
	});
});
