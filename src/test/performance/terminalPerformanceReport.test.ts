import { describe, expect, it } from "vitest";
import { createAgentTuiFixture } from "./terminalPerformanceFixture";
import {
	buildTerminalLaunchPerformanceReport,
	buildTerminalPerformanceReport,
	buildTerminalSurfacePerformanceReport,
	checkTerminalEndToEndPerformanceBudgets,
	checkTerminalPerformanceBudgets,
	formatTerminalPerformanceSummary,
	type TerminalLaunchPerformanceSamples,
	type TerminalPerformanceSamples,
	type TerminalSurfacePerformanceSamples,
} from "./terminalPerformanceReport";

function passingSamples(): TerminalPerformanceSamples {
	return {
		transport: "mocked-channel",
		fixture: {
			kind: "agent-tui",
			byteLength: 10 * 1024 * 1024,
			containsAnsi: true,
			containsUnicode: true,
			containsWideCharacters: true,
			containsCursorRedraw: true,
		},
		keyLatencyMs: Array.from({ length: 16 }, (_, index) => index + 1),
		imeCommitLatencyMs: [20, 21],
		revisitLatencyMs: [40],
		unloadedTimerDriftMs: [30],
		injectedLoadTimerDriftMs: [300],
		scrollLatencyMs: [50],
		restoreLatencyMs: [100],
		uiHeartbeatDriftMs: [40],
		workspaceSelectionLatencyMs: [60],
		rendererQueuedCodeUnits: [0],
		rendererPeakQueuedCodeUnits: [1024],
		rendererDroppedBacklogs: 0,
		snapshotResyncs: 0,
		rendererLongStallsOver100Ms: 0,
		throughputMiBPerSecond: 1,
		launchSource: "deterministic-fixture",
		launchProvider: "fixture",
		launchTotalMs: Array(30).fill(20),
		launchPhaseMs: {
			commandIngress: Array(30).fill(1),
			availabilityAndLock: Array(30).fill(1),
			durableCreateCommit: Array(30).fill(1),
			launchFileMaterialize: Array(30).fill(1),
			checkpointLookup: Array(30).fill(1),
			childEnvironment: Array(30).fill(1),
			ptyOpenAndSpawn: Array(30).fill(1),
			outputReaderReady: Array(30).fill(1),
			firstProviderByte: Array(30).fill(10),
			firstXtermParsed: Array(30).fill(15),
			firstPaint: Array(30).fill(20),
		},
	};
}

describe("Terminal Performance report contract", () => {
	it("Session起動reportをTerminal Surface負荷sampleから独立して生成する", () => {
		const samples: TerminalLaunchPerformanceSamples = {
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchTotalMs: Array.from({ length: 30 }, (_, index) => 20 + index),
			launchPhaseMs: passingSamples().launchPhaseMs,
		};

		const report = buildTerminalLaunchPerformanceReport(samples);

		expect(report.launchTotalMs).toEqual({
			count: 30,
			median: 34.5,
			p95: 48,
			max: 49,
		});
		expect(report.launchPhaseMs.firstPaint.count).toBe(30);
		expect(report).not.toHaveProperty("fixture");
	});

	it("Terminal Surface reportはSession起動sampleから独立して生成できる", () => {
		const {
			launchSource: _launchSource,
			launchProvider: _launchProvider,
			launchTotalMs: _launchTotalMs,
			launchPhaseMs: _launchPhaseMs,
			...surfaceSamples
		} = passingSamples();

		const report = buildTerminalSurfacePerformanceReport(
			surfaceSamples as TerminalSurfacePerformanceSamples,
		);

		expect(report.transport).toBe("mocked-channel");
		expect(report).not.toHaveProperty("launchTotalMs");
	});

	it("全budget fieldが揃っていないreportを拒否する", () => {
		const samples = passingSamples() as unknown as Record<string, unknown>;
		delete samples.restoreLatencyMs;

		expect(() => buildTerminalPerformanceReport(samples)).toThrow(
			"restoreLatencyMs",
		);
	});

	it("16 key未満、plain ASCII fixture、user dataを拒否する", () => {
		const tooFewKeys = passingSamples();
		tooFewKeys.keyLatencyMs = Array.from({ length: 15 }, () => 1);
		expect(() => buildTerminalPerformanceReport(tooFewKeys)).toThrow(
			"at least 16",
		);

		const plainAscii = passingSamples();
		plainAscii.fixture.containsUnicode = false;
		expect(() => buildTerminalPerformanceReport(plainAscii)).toThrow(
			"containsUnicode",
		);

		const withUserData = {
			...passingSamples(),
			agentSessionId: "secret-session-id",
		};
		expect(() => buildTerminalPerformanceReport(withUserData)).toThrow(
			"agentSessionId",
		);
	});

	it("同じraw sampleからquantile JSONとhuman summaryを生成する", () => {
		const samples = passingSamples();
		samples.keyLatencyMs = [
			1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 300,
		];

		const report = buildTerminalPerformanceReport(samples);

		expect(report.keyLatencyMs).toEqual({
			count: 16,
			median: 8.5,
			p95: 300,
			max: 300,
		});
		expect(formatTerminalPerformanceSummary(report)).toContain(
			"key latency median 8.50 ms / p95 300.00 ms / max 300.00 ms",
		);
		expect(JSON.stringify(report)).not.toContain("secret-session-id");
	});

	it("budget境界値を許可し超過とdrop/resync/stallを拒否する", () => {
		const boundary = passingSamples();
		boundary.keyLatencyMs = Array.from({ length: 16 }, () => 75);
		boundary.keyLatencyMs[15] = 300;
		boundary.revisitLatencyMs = [300];
		boundary.unloadedTimerDriftMs = [150];
		boundary.injectedLoadTimerDriftMs = [2500];
		boundary.scrollLatencyMs = [150];
		boundary.restoreLatencyMs = [1000];
		boundary.uiHeartbeatDriftMs = [150];
		boundary.workspaceSelectionLatencyMs = [300];
		boundary.rendererQueuedCodeUnits = [2_097_152];
		boundary.rendererPeakQueuedCodeUnits = [2_097_152];

		expect(
			checkTerminalPerformanceBudgets(buildTerminalPerformanceReport(boundary)),
		).toEqual([]);

		const exceeded = passingSamples();
		exceeded.keyLatencyMs = Array.from({ length: 16 }, () => 76);
		exceeded.rendererPeakQueuedCodeUnits = [2_097_153];
		exceeded.rendererDroppedBacklogs = 1;
		exceeded.snapshotResyncs = 1;
		exceeded.rendererLongStallsOver100Ms = 1;

		expect(
			checkTerminalPerformanceBudgets(buildTerminalPerformanceReport(exceeded)),
		).toEqual(
			expect.arrayContaining([
				expect.stringContaining("median key latency"),
				expect.stringContaining("renderer peak queue"),
				expect.stringContaining("dropped backlog"),
				expect.stringContaining("snapshot resync"),
				expect.stringContaining("renderer long stall"),
			]),
		);
	});

	it("mocked Channel reportをend-to-end性能の合否根拠として拒否する", () => {
		const report = buildTerminalPerformanceReport(passingSamples());

		expect(checkTerminalEndToEndPerformanceBudgets(report)).toEqual([
			"end-to-end performance requires tauri-ipc transport",
		]);
	});

	it("実Tauri reportは入力からpaintまで相関したphase sampleを必須とする", () => {
		const samples = passingSamples();
		samples.transport = "tauri-ipc";

		expect(() => buildTerminalPerformanceReport(samples)).toThrow(
			"inputTraceSamples",
		);
	});

	it("deterministic fixtureの30 warm runをtotalと全phaseのp50・p95・maxへ集計する", () => {
		const samples = {
			...passingSamples(),
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchTotalMs: Array.from({ length: 30 }, (_, index) => 20 + index),
		};
		samples.launchPhaseMs.firstProviderByte = Array(30).fill(10);
		samples.launchPhaseMs.firstXtermParsed = Array(30).fill(15);
		samples.launchPhaseMs.firstPaint = Array(30).fill(20);

		const report = buildTerminalPerformanceReport(samples) as ReturnType<
			typeof buildTerminalPerformanceReport
		> & {
			launchTotalMs: {
				count: number;
				median: number;
				p95: number;
				max: number;
			};
			launchSource: string;
			launchProvider: string;
			launchPhaseOwnership: Record<string, string>;
		};

		expect(report.launchTotalMs).toEqual({
			count: 30,
			median: 34.5,
			p95: 48,
			max: 49,
		});
		expect(report.launchPhaseMs.firstPaint).toEqual({
			count: 30,
			median: 20,
			p95: 20,
			max: 20,
		});
		expect(report).toMatchObject({
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchPhaseOwnership: {
				durableCreateCommit: "releash",
				firstProviderByte: "provider",
				firstXtermParsed: "end_to_end",
				firstPaint: "end_to_end",
			},
		});
	});

	it("launch run数不一致・相関点逆転・子phase合計がtotalを超えるreportを拒否する", () => {
		const mismatchedRuns = {
			...passingSamples(),
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchTotalMs: Array(31).fill(20),
		};
		expect(() => buildTerminalPerformanceReport(mismatchedRuns)).toThrow(
			"same 30 warm runs",
		);

		const reversedCorrelation = {
			...passingSamples(),
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchTotalMs: Array(30).fill(20),
		};
		reversedCorrelation.launchPhaseMs.firstProviderByte = Array(30).fill(16);
		reversedCorrelation.launchPhaseMs.firstXtermParsed = Array(30).fill(15);
		reversedCorrelation.launchPhaseMs.firstPaint = Array(30).fill(20);
		expect(() => buildTerminalPerformanceReport(reversedCorrelation)).toThrow(
			"correlation order",
		);

		const childrenExceedTotal = {
			...passingSamples(),
			launchSource: "deterministic-fixture",
			launchProvider: "fixture",
			launchTotalMs: Array(30).fill(10),
		};
		childrenExceedTotal.launchPhaseMs.availabilityAndLock = Array(30).fill(3);
		childrenExceedTotal.launchPhaseMs.firstProviderByte = Array(30).fill(1);
		childrenExceedTotal.launchPhaseMs.firstXtermParsed = Array(30).fill(5);
		childrenExceedTotal.launchPhaseMs.firstPaint = Array(30).fill(10);
		expect(() => buildTerminalPerformanceReport(childrenExceedTotal)).toThrow(
			"child phase sum",
		);
	});

	it("deterministic fixtureと実Provider観測のsource/provider組合せを混同しない", () => {
		const invalid = {
			...passingSamples(),
			launchSource: "deterministic-fixture",
			launchProvider: "codex",
		};

		expect(() => buildTerminalPerformanceReport(invalid)).toThrow(
			"launch source/provider",
		);
	});
});

describe("agent-TUI performance fixture", () => {
	it("10 MiB以上でANSI、Unicode、wide character、cursor redrawを含む", () => {
		const fixture = createAgentTuiFixture();

		expect(
			new TextEncoder().encode(fixture.data).byteLength,
		).toBeGreaterThanOrEqual(10 * 1024 * 1024);
		expect(fixture.descriptor).toMatchObject({
			kind: "agent-tui",
			containsAnsi: true,
			containsUnicode: true,
			containsWideCharacters: true,
			containsCursorRedraw: true,
		});
		expect(fixture.data).toContain("\u001b[");
		expect(fixture.data).toContain("日本語🙂");
		expect(fixture.data).toContain("\u001b[2K\r");
	});
});
