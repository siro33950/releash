import { describe, expect, it, vi } from "vitest";

const sdk = vi.hoisted(() => ({
	query: vi.fn(),
}));

vi.mock("@anthropic-ai/claude-agent-sdk", () => ({
	query: sdk.query,
}));

import { runClaudeListModelsProbe } from "./claude-sdk-bridge-list-models.mjs";

function makeProbeIo() {
	const stdout = [];
	const stderr = [];
	const exits = [];
	return {
		stdout,
		stderr,
		exits,
		writeStdout: (text) => stdout.push(text),
		writeStderr: (text) => stderr.push(text),
		exit: (code) => exits.push(code),
	};
}

describe("claude-sdk-bridge-list-models", () => {
	it("writes initializationResult models as stdout JSON", async () => {
		sdk.query.mockReturnValue({
			initializationResult: vi.fn(async () => ({
				models: [{ value: "claude-sonnet" }, { value: "claude-opus" }],
			})),
		});
		const io = makeProbeIo();

		await runClaudeListModelsProbe(io);

		expect(io.stdout.join("")).toBe(
			`${JSON.stringify({
				models: [{ value: "claude-sonnet" }, { value: "claude-opus" }],
			})}\n`,
		);
		expect(io.stderr).toEqual([]);
		expect(io.exits).toEqual([0]);
	});

	it("writes an empty models array when initializationResult omits models", async () => {
		sdk.query.mockReturnValue({
			initializationResult: vi.fn(async () => ({ sessionId: "s1" })),
		});
		const io = makeProbeIo();

		await runClaudeListModelsProbe(io);

		expect(io.stdout.join("")).toBe(`${JSON.stringify({ models: [] })}\n`);
		expect(io.stderr).toEqual([]);
		expect(io.exits).toEqual([0]);
	});

	it("writes stderr and exits non-zero on SDK failure", async () => {
		sdk.query.mockReturnValue({
			initializationResult: vi.fn(async () => {
				throw new Error("bad credentials");
			}),
		});
		const io = makeProbeIo();

		await runClaudeListModelsProbe(io);

		expect(io.stdout).toEqual([]);
		expect(io.stderr.join("")).toBe(
			"claude-list-models failed: bad credentials\n",
		);
		expect(io.exits).toEqual([1]);
	});
});
