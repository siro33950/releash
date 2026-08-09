import { describe, expect, it } from "vitest";
import {
	buildRealAppLoadReport,
	formatRealAppLoadSummary,
	summarizeRealAppLoadCadence,
	summarizeRealAppLoadSamples,
} from "./realAppLoadReport";

describe("summarizeRealAppLoadSamples", () => {
	it("median・p95・maxを昇順sortから計算する", () => {
		const summary = summarizeRealAppLoadSamples([50, 10, 30, 20, 40]);
		expect(summary.count).toBe(5);
		expect(summary.medianMs).toBe(30);
		expect(summary.p95Ms).toBe(50);
		expect(summary.maxMs).toBe(50);
	});

	it("空sampleはcount 0とNaNを返す", () => {
		const summary = summarizeRealAppLoadSamples([]);
		expect(summary.count).toBe(0);
		expect(Number.isNaN(summary.medianMs)).toBe(true);
	});
});

describe("summarizeRealAppLoadCadence", () => {
	it("到着間隔のnominalからの乖離を要約する", () => {
		const cadence = summarizeRealAppLoadCadence([0, 1000, 2100, 3000], 1000);
		expect(cadence.count).toBe(4);
		expect(cadence.nominalIntervalMs).toBe(1000);
		expect(cadence.medianDeviationMs).toBe(100);
		expect(cadence.p95DeviationMs).toBe(100);
		expect(cadence.maxDeviationMs).toBe(100);
	});

	it("到着1件以下では乖離をNaNにする", () => {
		const cadence = summarizeRealAppLoadCadence([0], 1000);
		expect(cadence.count).toBe(1);
		expect(Number.isNaN(cadence.medianDeviationMs)).toBe(true);
		expect(Number.isNaN(cadence.p95DeviationMs)).toBe(true);
	});
});

describe("buildRealAppLoadReport / formatRealAppLoadSummary", () => {
	it("入力を要約したreportと人間可読summaryを生成する", () => {
		const report = buildRealAppLoadReport({
			transport: "tauri-ipc",
			realApp: true,
			switches: {
				disableOutputFlowControl: true,
				disableTerminalJournal: false,
			},
			loadFixture: {
				kind: "sustained-agent-tui",
				frameBytes: 2048,
				frameIntervalMs: 16,
				durationMs: 15000,
				dsrIntervalMs: 1000,
			},
			typedKeyLatencyMs: [12, 20, 16],
			typedKeyOnDataToPublishMs: [5, 8, 6],
			typedKeyPublishToEchoVisibleMs: [7, 12, 10],
			imeCommitLatencyMs: [30],
			dsrReplyOnDataUnixMs: [0, 1010, 2000],
			workspaceSelectionLatencyMs: [80, 120],
			workspaceSelectionSplits: [
				{ bodyFirstMs: 12, contentFirstMs: 80 },
				{ bodyFirstMs: 9, contentFirstMs: 120 },
			],
			loadTimerDriftMs: 45,
			longtasks: { count: 4, totalMs: 260.5, maxMs: 120.25 },
			longtasksUnsupported: false,
			rendererPeakQueuedCodeUnits: 1024,
			rendererLongStallsOver100Ms: 1,
			rendererDroppedBacklogs: 0,
			snapshotResyncs: 0,
			caveats: ["typed-key paint attribution is approximate under load"],
		});

		expect(report.schemaVersion).toBe(1);
		expect(report.typedKeyUnderLoadMs.count).toBe(3);
		expect(report.typedKeyUnderLoadMs.medianMs).toBe(16);
		expect(report.dsrReplyCadence.count).toBe(3);
		expect(report.dsrReplyCadence.p95DeviationMs).toBe(10);
		expect(report.workspaceSelectionUnderLoadMs.maxMs).toBe(120);
		expect(report.longtasks).toEqual({
			count: 4,
			totalMs: 260.5,
			maxMs: 120.25,
		});
		expect(report.longtasksUnsupported).toBe(false);

		const summary = formatRealAppLoadSummary(report);
		expect(summary).toContain("realApp=true");
		expect(summary).toContain("disableOutputFlowControl");
		expect(summary).toContain("typed key under load: n=3");
		expect(summary).toContain("onData->publish (backend): n=3");
		expect(summary).toContain(
			"dsr reply cadence: n=3 nominal=1000ms medianDev=10.0ms p95Dev=10.0ms maxDev=10.0ms",
		);
		expect(summary).toContain(
			"splits (bodyFirst/contentFirst ms): 12/80, 9/120",
		);
		expect(summary).toContain("longtasks: n=4 total=260.5ms max=120.3ms");
		expect(summary).toContain("caveat: typed-key paint attribution");
	});

	it("longtask未対応環境ではlongtasksUnsupportedをreportとsummaryへ記録する", () => {
		const report = buildRealAppLoadReport({
			transport: "websocket",
			realApp: true,
			switches: {},
			loadFixture: {
				kind: "sustained-agent-tui",
				frameBytes: 2048,
				frameIntervalMs: 16,
				durationMs: 15000,
				dsrIntervalMs: 1000,
			},
			typedKeyLatencyMs: [12],
			typedKeyOnDataToPublishMs: [5],
			typedKeyPublishToEchoVisibleMs: [7],
			imeCommitLatencyMs: [30],
			dsrReplyOnDataUnixMs: [0, 1000],
			workspaceSelectionLatencyMs: [80],
			workspaceSelectionSplits: [{ bodyFirstMs: 12, contentFirstMs: 80 }],
			loadTimerDriftMs: 45,
			longtasks: { count: 0, totalMs: 0, maxMs: 0 },
			longtasksUnsupported: true,
			rendererPeakQueuedCodeUnits: 1024,
			rendererLongStallsOver100Ms: 0,
			rendererDroppedBacklogs: 0,
			snapshotResyncs: 0,
			caveats: [],
		});

		expect(report.longtasksUnsupported).toBe(true);

		const summary = formatRealAppLoadSummary(report);
		expect(summary).toContain("longtasks: unsupported");
		expect(summary).not.toContain("longtasks: n=");
	});
});
