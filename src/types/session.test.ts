import { describe, expect, it } from "vitest";
import type { MessagePart, ModelInfo } from "./session";
import {
	getModelInfoBackend,
	getModelInfoDisplayName,
	getModelInfoId,
	getTextContent,
	normalizeModelSelectionId,
	normalizePermissionMode,
} from "./session";

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

describe("ModelInfo helpers", () => {
	describe("getModelInfoDisplayName", () => {
		it("uses displayName before all fallback values", () => {
			expect(
				getModelInfoDisplayName({
					id: "entry-id",
					value: "legacy-value",
					model_id: "snake-model",
					modelId: "camel-model",
					display_name: "Snake Display",
					displayName: "Camel Display",
				}),
			).toBe("Camel Display");
		});

		it("falls back through display_name, value, modelId, model_id, id, and empty string", () => {
			expect(getModelInfoDisplayName({ display_name: "Snake Display" })).toBe(
				"Snake Display",
			);
			expect(getModelInfoDisplayName({ value: "legacy-value" })).toBe(
				"legacy-value",
			);
			expect(getModelInfoDisplayName({ modelId: "camel-model" })).toBe(
				"camel-model",
			);
			expect(getModelInfoDisplayName({ model_id: "snake-model" })).toBe(
				"snake-model",
			);
			expect(getModelInfoDisplayName({ id: "entry-id" })).toBe("entry-id");
			expect(getModelInfoDisplayName({})).toBe("");
		});
	});

	describe("getModelInfoId", () => {
		it("prefers explicit id", () => {
			expect(
				getModelInfoId({
					id: "codex:explicit",
					backend: "codex",
					model_id: "fallback",
				}),
			).toBe("codex:explicit");
		});

		it("builds backend:model_id when id is absent", () => {
			expect(getModelInfoId({ backend: "codex", model_id: "gpt-5.4" })).toBe(
				"codex:gpt-5.4",
			);
		});

		it("returns the model id alone when backend is absent", () => {
			expect(getModelInfoId({ modelId: "sonnet" })).toBe("sonnet");
		});

		it("returns an empty string when modelId is empty", () => {
			expect(getModelInfoId({ backend: "codex", modelId: "" })).toBe("");
			expect(getModelInfoId({ backend: "codex" })).toBe("");
		});
	});

	describe("getModelInfoBackend", () => {
		it("returns backend when present and empty string when absent", () => {
			expect(getModelInfoBackend({ backend: "codex" })).toBe("codex");
			expect(getModelInfoBackend({})).toBe("");
		});
	});

	describe("normalizeModelSelectionId", () => {
		const models: ModelInfo[] = [
			{ id: "claude:sonnet", backend: "claude", model_id: "sonnet" },
			{ id: "codex:gpt-5.4", backend: "codex", model_id: "gpt-5.4" },
		];

		it("returns empty string for empty selected values", () => {
			expect(normalizeModelSelectionId(models, "")).toBe("");
			expect(normalizeModelSelectionId(models, null)).toBe("");
			expect(normalizeModelSelectionId(models, undefined)).toBe("");
		});

		it("keeps an exact entry id match", () => {
			expect(normalizeModelSelectionId(models, "codex:gpt-5.4")).toBe(
				"codex:gpt-5.4",
			);
		});

		it("normalizes a raw model_id from persisted sessions to the entry id", () => {
			expect(normalizeModelSelectionId(models, "gpt-5.4")).toBe(
				"codex:gpt-5.4",
			);
		});

		it("passes through unmatched selections", () => {
			expect(normalizeModelSelectionId(models, "unknown-model")).toBe(
				"unknown-model",
			);
		});
	});
});
