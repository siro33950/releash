import { describe, expect, it } from "vitest";
import {
	formatGitError,
	formatRemoteServerError,
	formatUserFriendlyError,
} from "./errorHandler";

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

	it("formats TypeError with undefined reading", () => {
		const error = new TypeError(
			"Cannot read properties of undefined (reading 'path')",
		);
		expect(formatUserFriendlyError(error)).toBe(
			"Data is not ready yet. Please wait a moment.",
		);
	});

	it("formats network errors", () => {
		const error = new Error("failed to fetch");
		expect(formatUserFriendlyError(error)).toContain(
			"Network connection error",
		);
	});

	it("formats network request failed errors", () => {
		const error = new Error("Network request failed");
		expect(formatUserFriendlyError(error)).toContain(
			"Network connection error",
		);
	});

	it("formats Tauri command not found errors", () => {
		const error = new Error("command 'my_cmd' not found");
		expect(formatUserFriendlyError(error)).toBe(
			"System command not available. Please restart the app.",
		);
	});

	it("formats git errors", () => {
		const error = new Error("git commit failed");
		expect(formatGitError(error)).toContain("Git operation failed");
	});

	it("returns raw message when short and unrecognized", () => {
		const error = new Error("something went wrong");
		expect(formatUserFriendlyError(error)).toBe("Error: something went wrong");
	});

	it("returns generic message when error message exceeds 150 characters", () => {
		const error = new Error("x".repeat(200));
		expect(formatUserFriendlyError(error)).toBe(
			"An error occurred. Please check the console for details.",
		);
	});

	it("formats remote server errors via formatRemoteServerError", () => {
		const error = new TypeError("Cannot read properties of null");
		expect(formatRemoteServerError(error)).toBe(
			"Failed to manage remote server. Please try again.",
		);
	});
});
