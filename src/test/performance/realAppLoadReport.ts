export interface RealAppLoadSampleSummary {
	count: number;
	medianMs: number;
	p95Ms: number;
	maxMs: number;
}

export interface RealAppLoadCadenceSummary {
	count: number;
	nominalIntervalMs: number;
	medianDeviationMs: number;
	p95DeviationMs: number;
	maxDeviationMs: number;
}

export interface RealAppLoadLongtaskSummary {
	count: number;
	totalMs: number;
	maxMs: number;
}

export interface RealAppLoadReportInput {
	transport: string;
	realApp: boolean;
	switches: Record<string, boolean>;
	loadFixture: {
		kind: string;
		frameBytes: number;
		frameIntervalMs: number;
		durationMs: number;
		dsrIntervalMs: number;
	};
	typedKeyLatencyMs: number[];
	typedKeyOnDataToPublishMs: number[];
	typedKeyPublishToEchoVisibleMs: number[];
	imeCommitLatencyMs: number[];
	dsrReplyOnDataUnixMs: number[];
	workspaceSelectionLatencyMs: number[];
	workspaceSelectionSplits: Array<{
		bodyFirstMs: number | null;
		contentFirstMs: number | null;
	}>;
	loadTimerDriftMs: number;
	longtasks: RealAppLoadLongtaskSummary;
	longtasksUnsupported: boolean;
	rendererPeakQueuedCodeUnits: number;
	rendererLongStallsOver100Ms: number;
	rendererDroppedBacklogs: number;
	snapshotResyncs: number;
	caveats: string[];
}

export interface RealAppLoadReport {
	schemaVersion: number;
	transport: string;
	realApp: boolean;
	switches: Record<string, boolean>;
	loadFixture: RealAppLoadReportInput["loadFixture"];
	typedKeyUnderLoadMs: RealAppLoadSampleSummary;
	typedKeyOnDataToPublishMs: RealAppLoadSampleSummary;
	typedKeyPublishToEchoVisibleMs: RealAppLoadSampleSummary;
	imeCommitUnderLoadMs: RealAppLoadSampleSummary;
	dsrReplyCadence: RealAppLoadCadenceSummary;
	workspaceSelectionUnderLoadMs: RealAppLoadSampleSummary;
	workspaceSelectionSplits: RealAppLoadReportInput["workspaceSelectionSplits"];
	loadTimerDriftMs: number;
	longtasks: RealAppLoadLongtaskSummary;
	longtasksUnsupported: boolean;
	rendererPeakQueuedCodeUnits: number;
	rendererLongStallsOver100Ms: number;
	rendererDroppedBacklogs: number;
	snapshotResyncs: number;
	caveats: string[];
}

export function summarizeRealAppLoadSamples(
	samples: number[],
): RealAppLoadSampleSummary {
	if (samples.length === 0) {
		return {
			count: 0,
			medianMs: Number.NaN,
			p95Ms: Number.NaN,
			maxMs: Number.NaN,
		};
	}
	const sorted = [...samples].sort((left, right) => left - right);
	const percentile = (ratio: number): number =>
		sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * ratio) - 1)];
	return {
		count: sorted.length,
		medianMs: percentile(0.5),
		p95Ms: percentile(0.95),
		maxMs: sorted[sorted.length - 1],
	};
}

export function summarizeRealAppLoadCadence(
	arrivalUnixMs: number[],
	nominalIntervalMs: number,
): RealAppLoadCadenceSummary {
	const intervals: number[] = [];
	for (let index = 1; index < arrivalUnixMs.length; index += 1) {
		intervals.push(arrivalUnixMs[index] - arrivalUnixMs[index - 1]);
	}
	const deviations = intervals.map((interval) =>
		Math.abs(interval - nominalIntervalMs),
	);
	const summary = summarizeRealAppLoadSamples(deviations);
	return {
		count: arrivalUnixMs.length,
		nominalIntervalMs,
		medianDeviationMs: summary.medianMs,
		p95DeviationMs: summary.p95Ms,
		maxDeviationMs: summary.maxMs,
	};
}

export function buildRealAppLoadReport(
	input: RealAppLoadReportInput,
): RealAppLoadReport {
	return {
		schemaVersion: 1,
		transport: input.transport,
		realApp: input.realApp,
		switches: input.switches,
		loadFixture: input.loadFixture,
		typedKeyUnderLoadMs: summarizeRealAppLoadSamples(input.typedKeyLatencyMs),
		typedKeyOnDataToPublishMs: summarizeRealAppLoadSamples(
			input.typedKeyOnDataToPublishMs,
		),
		typedKeyPublishToEchoVisibleMs: summarizeRealAppLoadSamples(
			input.typedKeyPublishToEchoVisibleMs,
		),
		imeCommitUnderLoadMs: summarizeRealAppLoadSamples(input.imeCommitLatencyMs),
		dsrReplyCadence: summarizeRealAppLoadCadence(
			input.dsrReplyOnDataUnixMs,
			input.loadFixture.dsrIntervalMs,
		),
		workspaceSelectionUnderLoadMs: summarizeRealAppLoadSamples(
			input.workspaceSelectionLatencyMs,
		),
		workspaceSelectionSplits: input.workspaceSelectionSplits,
		loadTimerDriftMs: input.loadTimerDriftMs,
		longtasks: { ...input.longtasks },
		longtasksUnsupported: input.longtasksUnsupported,
		rendererPeakQueuedCodeUnits: input.rendererPeakQueuedCodeUnits,
		rendererLongStallsOver100Ms: input.rendererLongStallsOver100Ms,
		rendererDroppedBacklogs: input.rendererDroppedBacklogs,
		snapshotResyncs: input.snapshotResyncs,
		caveats: input.caveats,
	};
}

export function formatRealAppLoadSummary(report: RealAppLoadReport): string {
	const formatSummary = (summary: RealAppLoadSampleSummary): string =>
		summary.count === 0
			? "no samples"
			: `n=${summary.count} median=${summary.medianMs.toFixed(1)}ms p95=${summary.p95Ms.toFixed(1)}ms max=${summary.maxMs.toFixed(1)}ms`;
	const activeSwitches = Object.entries(report.switches)
		.filter(([, enabled]) => enabled)
		.map(([name]) => name);
	return [
		`transport: ${report.transport} (realApp=${report.realApp})`,
		`switches: ${activeSwitches.length ? activeSwitches.join(", ") : "none (production path)"}`,
		`load fixture: ${report.loadFixture.kind} frame=${report.loadFixture.frameBytes}B interval=${report.loadFixture.frameIntervalMs}ms duration=${report.loadFixture.durationMs}ms`,
		`typed key under load: ${formatSummary(report.typedKeyUnderLoadMs)}`,
		`  - onData->publish (backend): ${formatSummary(report.typedKeyOnDataToPublishMs)}`,
		`  - publish->echo visible (delivery+parse): ${formatSummary(report.typedKeyPublishToEchoVisibleMs)}`,
		`ime commit under load: ${formatSummary(report.imeCommitUnderLoadMs)}`,
		`dsr reply cadence: n=${report.dsrReplyCadence.count} nominal=${report.dsrReplyCadence.nominalIntervalMs}ms medianDev=${report.dsrReplyCadence.medianDeviationMs.toFixed(1)}ms p95Dev=${report.dsrReplyCadence.p95DeviationMs.toFixed(1)}ms maxDev=${report.dsrReplyCadence.maxDeviationMs.toFixed(1)}ms`,
		`workspace selection under load: ${formatSummary(report.workspaceSelectionUnderLoadMs)}`,
		`  - splits (bodyFirst/contentFirst ms): ${report.workspaceSelectionSplits
			.map(
				(split) =>
					`${split.bodyFirstMs === null ? "-" : split.bodyFirstMs.toFixed(0)}/${split.contentFirstMs === null ? "-" : split.contentFirstMs.toFixed(0)}`,
			)
			.join(", ")}`,
		`load timer drift: ${report.loadTimerDriftMs.toFixed(1)}ms`,
		report.longtasksUnsupported
			? "longtasks: unsupported (PerformanceObserver longtask is unavailable in this WebView)"
			: `longtasks: n=${report.longtasks.count} total=${report.longtasks.totalMs.toFixed(1)}ms max=${report.longtasks.maxMs.toFixed(1)}ms`,
		`renderer peak queue: ${report.rendererPeakQueuedCodeUnits} code units, longStalls>100ms: ${report.rendererLongStallsOver100Ms}, droppedBacklogs: ${report.rendererDroppedBacklogs}, snapshotResyncs: ${report.snapshotResyncs}`,
		...report.caveats.map((caveat) => `caveat: ${caveat}`),
	].join("\n");
}
