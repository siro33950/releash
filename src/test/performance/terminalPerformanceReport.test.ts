import { describe, expect, it } from "vitest";
import {
	buildTerminalLaunchPerformanceReport,
	buildTerminalSurfacePerformanceReport,
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
});
