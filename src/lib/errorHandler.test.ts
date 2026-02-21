import { describe, expect, it } from "vitest";
import { formatGitError, formatUserFriendlyError } from "./errorHandler";

describe("errorHandler", () => {
	it("formats TypeError with null reading", () => {
		const error = new TypeError(
			"Cannot read properties of null (reading 'path')",
		);
		expect(formatUserFriendlyError(error)).toBe(
			"An unexpected error occurred. Please try again.",
		);
	});

	it("includes operation context", () => {
		const error = new TypeError("Cannot read properties of null");
		expect(formatUserFriendlyError(error, { operation: "load data" })).toBe(
			"Failed to load data. Please try again.",
		);
	});

	it("formats network errors", () => {
		const error = new Error("fetch failed");
		expect(formatUserFriendlyError(error)).toContain(
			"Network connection error",
		);
	});

	it("formats git errors", () => {
		const error = new Error("git commit failed");
		expect(formatGitError(error)).toContain("Git operation failed");
	});
});
