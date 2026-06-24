import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
	buildResultTurnCompletion,
	buildSystemPromptOption,
	buildTurnCompleteMessage,
	consumeQueryInitTelemetryMessage,
	createQueryInitTelemetryState,
	markQueryInitTelemetryStarted,
	rollbackResumeSessionIdAfterInterrupt,
	shouldContinueBridgeLoopAfterQueryEnd,
	shouldResolvePromptForCurrentQuery,
	withTurnToken,
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

describe("query init telemetry helpers", () => {
	it("measures from prompt yield turn start and excludes inter-turn idle", () => {
		const state = createQueryInitTelemetryState();

		markQueryInitTelemetryStarted(state, 5_000);

		expect(consumeQueryInitTelemetryMessage(state, 5_125)).toEqual({
			type: "telemetry",
			metric: "query_init",
			duration_ms: 125,
		});
		expect(consumeQueryInitTelemetryMessage(state, 5_250)).toBeNull();
	});

	it("excludes initial and inter-turn prompt wait idle before prompt yield", () => {
		const state = createQueryInitTelemetryState();
		const initialQueryCreatedAtMs = 1_000;
		const initialPromptYieldedAtMs = initialQueryCreatedAtMs + 4_000;

		markQueryInitTelemetryStarted(state, initialPromptYieldedAtMs);

		expect(
			consumeQueryInitTelemetryMessage(state, initialPromptYieldedAtMs + 125),
		).toEqual({
			type: "telemetry",
			metric: "query_init",
			duration_ms: 125,
		});

		const nextQueryCreatedAtMs = 8_000;
		const nextPromptYieldedAtMs = nextQueryCreatedAtMs + 12_000;

		markQueryInitTelemetryStarted(state, nextPromptYieldedAtMs);

		expect(
			consumeQueryInitTelemetryMessage(state, nextPromptYieldedAtMs + 80),
		).toEqual({
			type: "telemetry",
			metric: "query_init",
			duration_ms: 80,
		});
	});

	it("falls back to zero duration when the prompt yield timestamp is missing", () => {
		const state = createQueryInitTelemetryState();

		expect(consumeQueryInitTelemetryMessage(state, 9_000)).toEqual({
			type: "telemetry",
			metric: "query_init",
			duration_ms: 0,
		});
	});
});

describe("buildResultTurnCompletion", () => {
	it("treats a result delivered after abort as interrupted", () => {
		expect(
			buildResultTurnCompletion({
				sessionId: "sdk-interrupted",
				currentSessionId: "sdk-interrupted",
				hasErrors: false,
				wasAborted: true,
				turnToken: "agent-message-1",
			}),
		).toEqual({
			message: {
				type: "turn_complete",
				session_id: "sdk-interrupted",
				exit_code: 0,
				interrupted: true,
				turn_token: "agent-message-1",
			},
			exitCode: 0,
			completedSessionIdForResume: null,
		});
	});

	it("keeps a successful non-aborted result reusable for resume", () => {
		expect(
			buildResultTurnCompletion({
				sessionId: "sdk-clean",
				currentSessionId: "sdk-current",
				hasErrors: false,
				wasAborted: false,
			}),
		).toEqual({
			message: {
				type: "turn_complete",
				session_id: "sdk-clean",
				exit_code: 0,
			},
			exitCode: 0,
			completedSessionIdForResume: "sdk-clean",
		});
	});

	it("does not reuse errored result sessions for resume", () => {
		expect(
			buildResultTurnCompletion({
				sessionId: "sdk-error",
				currentSessionId: "sdk-current",
				hasErrors: true,
				wasAborted: false,
			}),
		).toEqual({
			message: {
				type: "turn_complete",
				session_id: "sdk-error",
				exit_code: 1,
			},
			exitCode: 1,
			completedSessionIdForResume: null,
		});
	});
});

describe("buildTurnCompleteMessage", () => {
	it("marks interrupted completions explicitly and echoes turn token", () => {
		expect(
			buildTurnCompleteMessage({
				sessionId: "sdk-interrupted",
				exitCode: 0,
				interrupted: true,
				turnToken: "agent-message-1",
			}),
		).toEqual({
			type: "turn_complete",
			session_id: "sdk-interrupted",
			exit_code: 0,
			interrupted: true,
			turn_token: "agent-message-1",
		});
	});

	it("keeps normal completions backward compatible", () => {
		expect(
			buildTurnCompleteMessage({
				sessionId: "sdk-ok",
				exitCode: 0,
			}),
		).toEqual({
			type: "turn_complete",
			session_id: "sdk-ok",
			exit_code: 0,
		});
	});
});

describe("rollbackResumeSessionIdAfterInterrupt", () => {
	it("rolls back to the last successful result session", () => {
		expect(
			rollbackResumeSessionIdAfterInterrupt({
				lastResultSessionId: "sdk-clean",
			}),
		).toBe("sdk-clean");
	});

	it("starts fresh when the first turn was interrupted", () => {
		expect(
			rollbackResumeSessionIdAfterInterrupt({
				lastResultSessionId: null,
			}),
		).toBeNull();
	});
});

describe("withTurnToken", () => {
	it("adds turn_token when present", () => {
		expect(withTurnToken({ type: "assistant" }, "agent-message-1")).toEqual({
			type: "assistant",
			turn_token: "agent-message-1",
		});
	});

	it("does not change tokenless messages", () => {
		const message = { type: "assistant" };
		expect(withTurnToken(message, null)).toBe(message);
	});
});

describe("claude bridge permission requests", () => {
	it("emits permission_request with the current turn token", () => {
		const source = readFileSync(
			join(process.cwd(), "src-tauri/resources/claude-sdk-bridge.mjs"),
			"utf8",
		);

		expect(source).toMatch(
			/emit\(\s*withTurnToken\(\s*\{\s*type: "permission_request"[\s\S]*?\},\s*currentTurnToken,\s*\),\s*\);/,
		);
	});
});

describe("claude bridge query init telemetry", () => {
	it("emits query_init telemetry with the current turn token", () => {
		const bridgeSource = readFileSync(
			join(process.cwd(), "src-tauri/resources/claude-sdk-bridge.mjs"),
			"utf8",
		);
		const utilsSource = readFileSync(
			join(process.cwd(), "src-tauri/resources/bridge-utils.mjs"),
			"utf8",
		);

		const telemetryFactory = utilsSource.match(
			/return\s*\{\s*type: "telemetry",\s*metric: "query_init",\s*duration_ms: Math\.max\([\s\S]*?\),\s*\};/,
		)?.[0];
		expect(telemetryFactory).toBeTruthy();
		expect(telemetryFactory).toContain('type: "telemetry"');
		expect(telemetryFactory).toContain('metric: "query_init"');
		expect(telemetryFactory).toContain("duration_ms");
		expect(telemetryFactory).not.toMatch(/\b(prompt|content)\b/);

		expect(bridgeSource).toMatch(
			/for await \(const message of currentQuery\) \{[\s\S]*?const queryInitTelemetry = consumeQueryInitTelemetryMessage\([\s\S]*?\);[\s\S]*?if \(queryInitTelemetry\) \{[\s\S]*?emit\(\s*withTurnToken\(\s*queryInitTelemetry,\s*currentTurnToken,\s*\),\s*\);/,
		);
		const emitCall = bridgeSource.match(
			/emit\(\s*withTurnToken\(\s*queryInitTelemetry,\s*currentTurnToken,\s*\),\s*\);/,
		)?.[0];
		expect(emitCall).toBeTruthy();
		expect(emitCall).toContain("currentTurnToken");
		expect(utilsSource).toMatch(/turn_token: turnToken/);
		expect(emitCall).not.toMatch(/\b(prompt|content|body)\b/);
	});
});
