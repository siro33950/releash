import type { TerminalOutputSchedulerMetrics } from "@/lib/terminalOutputScheduler";

export interface TerminalInputPerformancePoints {
	on_data?: number;
	channel_receive?: number;
	xterm_parsed?: number;
	paint?: number;
}

export interface TerminalLongtaskWindowState {
	count: number;
	totalMs: number;
	maxMs: number;
}

export interface TerminalPerformanceWindowState {
	phases: Record<string, number[]>;
	inputPoints: Record<string, TerminalInputPerformancePoints>;
	rendererMetrics: TerminalOutputSchedulerMetrics;
	rendererQueuedCodeUnits: number[];
	rendererPeakQueuedCodeUnits: number[];
	maxHeartbeatDriftMs: number;
	launchOrigins: Record<string, number>;
	longtasks: TerminalLongtaskWindowState;
	longtasksUnsupported: boolean;
}

declare global {
	interface Window {
		__RELEASH_TERMINAL_PERFORMANCE_STATE__?: TerminalPerformanceWindowState;
	}
}

// WKWebViewはlongtask entry未対応のため、supportedEntryTypesで確認できた
// 場合のみ観測する。未対応環境ではlongtasksUnsupportedをtrueのまま残す。
function installLongtaskObserver(state: TerminalPerformanceWindowState): void {
	if (
		typeof PerformanceObserver === "undefined" ||
		!PerformanceObserver.supportedEntryTypes?.includes("longtask")
	) {
		return;
	}
	try {
		const observer = new PerformanceObserver((list) => {
			for (const entry of list.getEntries()) {
				state.longtasks.count += 1;
				state.longtasks.totalMs += entry.duration;
				state.longtasks.maxMs = Math.max(state.longtasks.maxMs, entry.duration);
			}
		});
		observer.observe({ type: "longtask", buffered: true });
		state.longtasksUnsupported = false;
	} catch {
		state.longtasksUnsupported = true;
	}
}

export function installPerformanceCollector(): void {
	if (window.__RELEASH_TERMINAL_PERFORMANCE_STATE__) return;
	const state: TerminalPerformanceWindowState = {
		phases: {},
		inputPoints: {},
		rendererMetrics: {
			currentQueuedCodeUnits: 0,
			peakQueuedCodeUnits: 0,
			writeCount: 0,
			longStallsOver100Ms: 0,
			droppedBacklogs: 0,
			snapshotResyncs: 0,
		},
		rendererQueuedCodeUnits: [0],
		rendererPeakQueuedCodeUnits: [0],
		maxHeartbeatDriftMs: 0,
		launchOrigins: {},
		longtasks: { count: 0, totalMs: 0, maxMs: 0 },
		longtasksUnsupported: true,
	};
	window.__RELEASH_TERMINAL_PERFORMANCE_STATE__ = state;
	installLongtaskObserver(state);
	window.__RELEASH_TERMINAL_PERFORMANCE__ = {
		recordPhase: (phase, durationMs) => {
			const samples = state.phases[phase] ?? [];
			samples.push(durationMs);
			state.phases[phase] = samples;
		},
		recordInputPoint: (sequence, phase, atUnixMs) => {
			const key = String(sequence);
			const points = state.inputPoints[key] ?? {};
			points[phase] = atUnixMs;
			state.inputPoints[key] = points;
		},
		recordRendererMetrics: (metrics) => {
			state.rendererMetrics = metrics;
			state.rendererQueuedCodeUnits.push(metrics.currentQueuedCodeUnits);
			state.rendererPeakQueuedCodeUnits.push(metrics.peakQueuedCodeUnits);
		},
		takeLaunchOrigin: (agentSessionId) => {
			const origin = state.launchOrigins[agentSessionId];
			delete state.launchOrigins[agentSessionId];
			return origin ?? null;
		},
	};
	let expected = performance.now() + 10;
	window.setInterval(() => {
		const current = performance.now();
		state.maxHeartbeatDriftMs = Math.max(
			state.maxHeartbeatDriftMs,
			current - expected,
		);
		expected = current + 10;
	}, 10);
}
