import { afterEach, describe, expect, it, vi } from "vitest";
import {
	reportTerminalInputPerformancePoint,
	reportTerminalPerformancePhase,
	reportTerminalRendererMetrics,
	shouldReportTerminalSnapshotLaunchPhase,
	takeTerminalLaunchPerformanceOrigin,
} from "./terminalPerformanceProbe";

describe("Terminal performance probe", () => {
	afterEach(() => {
		delete window.__RELEASH_TERMINAL_PERFORMANCE__;
	});

	it("通常起動ではno-opになる", () => {
		expect(() =>
			reportTerminalPerformancePhase("first_xterm_parsed", 12),
		).not.toThrow();
		expect(() =>
			reportTerminalRendererMetrics({
				currentQueuedCodeUnits: 0,
				peakQueuedCodeUnits: 10,
				writeCount: 1,
				longStallsOver100Ms: 0,
				droppedBacklogs: 0,
				snapshotResyncs: 0,
			}),
		).not.toThrow();
	});

	it("opt-in collectorへ列挙済みphaseと数値metricだけを渡す", () => {
		vi.spyOn(Date, "now").mockReturnValue(1_234);
		const recordPhase = vi.fn();
		const recordRendererMetrics = vi.fn();
		const recordInputPoint = vi.fn();
		window.__RELEASH_TERMINAL_PERFORMANCE__ = {
			recordInputPoint,
			recordPhase,
			recordRendererMetrics,
		};

		reportTerminalPerformancePhase("first_paint", 23);
		reportTerminalInputPerformancePoint(4, "channel_receive");
		reportTerminalRendererMetrics({
			currentQueuedCodeUnits: 5,
			peakQueuedCodeUnits: 10,
			writeCount: 2,
			longStallsOver100Ms: 0,
			droppedBacklogs: 0,
			snapshotResyncs: 0,
		});

		expect(recordPhase).toHaveBeenCalledWith("first_paint", 23);
		expect(recordInputPoint).toHaveBeenCalledWith(4, "channel_receive", 1_234);
		expect(recordRendererMetrics).toHaveBeenCalledWith({
			currentQueuedCodeUnits: 5,
			peakQueuedCodeUnits: 10,
			writeCount: 2,
			longStallsOver100Ms: 0,
			droppedBacklogs: 0,
			snapshotResyncs: 0,
		});
	});

	it("Session作成開始時刻を対象Sessionの最初のattachだけへ引き継ぐ", () => {
		const takeLaunchOrigin = vi
			.fn()
			.mockReturnValueOnce(42)
			.mockReturnValueOnce(null);
		window.__RELEASH_TERMINAL_PERFORMANCE__ = {
			recordInputPoint: vi.fn(),
			recordPhase: vi.fn(),
			recordRendererMetrics: vi.fn(),
			takeLaunchOrigin,
		};

		expect(takeTerminalLaunchPerformanceOrigin("session-1")).toBe(42);
		expect(takeTerminalLaunchPerformanceOrigin("session-1")).toBeNull();
		expect(takeLaunchOrigin).toHaveBeenNthCalledWith(1, "session-1");
		expect(takeLaunchOrigin).toHaveBeenNthCalledWith(2, "session-1");
	});

	it("sequence 0の初期replayをProvider first outputとして扱わない", () => {
		expect(shouldReportTerminalSnapshotLaunchPhase(0)).toBe(false);
		expect(shouldReportTerminalSnapshotLaunchPhase(1)).toBe(true);
	});
});
