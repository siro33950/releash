import { describe, expect, it } from "vitest";
import {
	buildSystemPromptOption,
	shouldContinueBridgeLoopAfterQueryEnd,
	shouldResolvePromptForCurrentQuery,
} from "./bridge-utils.mjs";

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

describe("shouldContinueBridgeLoopAfterQueryEnd", () => {
	it("continues after a normal SDK iterator completion", () => {
		expect(
			shouldContinueBridgeLoopAfterQueryEnd({
				closed: false,
				turnExitCode: 0,
			}),
		).toBe(true);
	});

	it("stops after an explicit close command", () => {
		expect(
			shouldContinueBridgeLoopAfterQueryEnd({
				closed: true,
				turnExitCode: 0,
			}),
		).toBe(false);
	});

	it("stops after a failed result", () => {
		expect(
			shouldContinueBridgeLoopAfterQueryEnd({
				closed: false,
				turnExitCode: 1,
			}),
		).toBe(false);
	});

	it("stops when the SDK iterator ends before a completed turn", () => {
		expect(
			shouldContinueBridgeLoopAfterQueryEnd({
				closed: false,
				turnExitCode: null,
			}),
		).toBe(false);
	});
});

describe("shouldResolvePromptForCurrentQuery", () => {
	it("resolves a waiting prompt before the active query completes", () => {
		expect(
			shouldResolvePromptForCurrentQuery({
				hasPendingPromptResolver: true,
				completedResultForCurrentQuery: false,
			}),
		).toBe(true);
	});

	it("queues prompts after the active query has completed its result", () => {
		expect(
			shouldResolvePromptForCurrentQuery({
				hasPendingPromptResolver: true,
				completedResultForCurrentQuery: true,
			}),
		).toBe(false);
	});

	it("queues prompts when no active prompt resolver is waiting", () => {
		expect(
			shouldResolvePromptForCurrentQuery({
				hasPendingPromptResolver: false,
				completedResultForCurrentQuery: false,
			}),
		).toBe(false);
	});
});
