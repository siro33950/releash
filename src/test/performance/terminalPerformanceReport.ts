export interface TerminalPerformanceFixtureDescriptor {
	kind: "agent-tui";
	byteLength: number;
	containsAnsi: boolean;
	containsUnicode: boolean;
	containsWideCharacters: boolean;
	containsCursorRedraw: boolean;
}

export interface TerminalPerformanceSamples {
	transport: "mocked-channel" | "tauri-ipc";
	inputTraceSamples?: TerminalInputTraceSample[];
	fixture: TerminalPerformanceFixtureDescriptor;
	keyLatencyMs: number[];
	imeCommitLatencyMs: number[];
	revisitLatencyMs: number[];
	unloadedTimerDriftMs: number[];
	injectedLoadTimerDriftMs: number[];
	scrollLatencyMs: number[];
	restoreLatencyMs: number[];
	uiHeartbeatDriftMs: number[];
	workspaceSelectionLatencyMs: number[];
	rendererQueuedCodeUnits: number[];
	rendererPeakQueuedCodeUnits: number[];
	rendererDroppedBacklogs: number;
	snapshotResyncs: number;
	rendererLongStallsOver100Ms: number;
	throughputMiBPerSecond: number;
	launchSource: "deterministic-fixture" | "provider-observation";
	launchProvider: "fixture" | "tui-fixture" | "claude" | "codex";
	launchTotalMs: number[];
	launchPhaseMs: Record<TerminalLaunchPhase, number[]>;
}

export type TerminalSurfacePerformanceSamples = Omit<
	TerminalPerformanceSamples,
	"launchSource" | "launchProvider" | "launchTotalMs" | "launchPhaseMs"
>;

export type TerminalLaunchPerformanceSamples = Pick<
	TerminalPerformanceSamples,
	"launchSource" | "launchProvider" | "launchTotalMs" | "launchPhaseMs"
>;

export interface TerminalInputTraceSample {
	onDataToCommandIngressMs: number;
	commandIngressToAdmissionMs: number;
	admissionToWriterEnqueueMs: number;
	writerEnqueueToOutputReadMs: number;
	outputReadToModelApplyMs: number;
	modelApplyToEventPublishMs: number;
	eventPublishToChannelReceiveMs: number;
	channelReceiveToXtermParsedMs: number;
	xtermParsedToPaintMs: number;
	totalMs: number;
}

export type TerminalInputTracePhase = Exclude<
	keyof TerminalInputTraceSample,
	"totalMs"
>;

export type TerminalLaunchPhase =
	| "commandIngress"
	| "availabilityAndLock"
	| "durableCreateCommit"
	| "launchFileMaterialize"
	| "checkpointLookup"
	| "childEnvironment"
	| "ptyOpenAndSpawn"
	| "outputReaderReady"
	| "firstProviderByte"
	| "firstXtermParsed"
	| "firstPaint";

export type TerminalLaunchPhaseOwnership =
	| "releash"
	| "provider"
	| "end_to_end";

const TERMINAL_LAUNCH_PHASES: TerminalLaunchPhase[] = [
	"commandIngress",
	"availabilityAndLock",
	"durableCreateCommit",
	"launchFileMaterialize",
	"checkpointLookup",
	"childEnvironment",
	"ptyOpenAndSpawn",
	"outputReaderReady",
	"firstProviderByte",
	"firstXtermParsed",
	"firstPaint",
];

const TERMINAL_LAUNCH_PHASE_OWNERSHIP: Record<
	TerminalLaunchPhase,
	TerminalLaunchPhaseOwnership
> = {
	commandIngress: "releash",
	availabilityAndLock: "releash",
	durableCreateCommit: "releash",
	launchFileMaterialize: "releash",
	checkpointLookup: "releash",
	childEnvironment: "releash",
	ptyOpenAndSpawn: "releash",
	outputReaderReady: "releash",
	firstProviderByte: "provider",
	firstXtermParsed: "end_to_end",
	firstPaint: "end_to_end",
};

interface Distribution {
	count: number;
	median: number;
	p95: number;
	max: number;
}

export interface TerminalSurfacePerformanceReport {
	schemaVersion: 1;
	transport: TerminalSurfacePerformanceSamples["transport"];
	inputPhaseMs?: Record<TerminalInputTracePhase, Distribution>;
	fixture: TerminalPerformanceFixtureDescriptor;
	keyLatencyMs: Distribution;
	imeCommitLatencyMs: Distribution;
	revisitLatencyMs: Distribution;
	unloadedTimerDriftMs: Distribution;
	injectedLoadTimerDriftMs: Distribution;
	scrollLatencyMs: Distribution;
	restoreLatencyMs: Distribution;
	uiHeartbeatDriftMs: Distribution;
	workspaceSelectionLatencyMs: Distribution;
	rendererQueuedCodeUnits: Distribution;
	rendererPeakQueuedCodeUnits: Distribution;
	rendererDroppedBacklogs: number;
	snapshotResyncs: number;
	rendererLongStallsOver100Ms: number;
	throughputMiBPerSecond: number;
}

export interface TerminalLaunchPerformanceReport {
	schemaVersion: 1;
	launchSource: TerminalPerformanceSamples["launchSource"];
	launchProvider: TerminalPerformanceSamples["launchProvider"];
	launchTotalMs: Distribution;
	launchPhaseMs: Record<TerminalLaunchPhase, Distribution>;
	launchPhaseOwnership: Record<
		TerminalLaunchPhase,
		TerminalLaunchPhaseOwnership
	>;
}

const REQUIRED_ARRAY_FIELDS = [
	"keyLatencyMs",
	"imeCommitLatencyMs",
	"revisitLatencyMs",
	"unloadedTimerDriftMs",
	"injectedLoadTimerDriftMs",
	"scrollLatencyMs",
	"restoreLatencyMs",
	"uiHeartbeatDriftMs",
	"workspaceSelectionLatencyMs",
	"rendererQueuedCodeUnits",
	"rendererPeakQueuedCodeUnits",
] as const;

const REQUIRED_NUMBER_FIELDS = [
	"rendererDroppedBacklogs",
	"snapshotResyncs",
	"rendererLongStallsOver100Ms",
	"throughputMiBPerSecond",
] as const;

const FORBIDDEN_USER_DATA_FIELDS = new Set([
	"path",
	"input",
	"inputText",
	"agentSessionId",
	"providerSessionId",
]);

const TERMINAL_INPUT_TRACE_PHASES: TerminalInputTracePhase[] = [
	"onDataToCommandIngressMs",
	"commandIngressToAdmissionMs",
	"admissionToWriterEnqueueMs",
	"writerEnqueueToOutputReadMs",
	"outputReadToModelApplyMs",
	"modelApplyToEventPublishMs",
	"eventPublishToChannelReceiveMs",
	"channelReceiveToXtermParsedMs",
	"xtermParsedToPaintMs",
];

function assertNoUserData(value: unknown): void {
	if (!value || typeof value !== "object") return;
	for (const [key, child] of Object.entries(value)) {
		if (FORBIDDEN_USER_DATA_FIELDS.has(key)) {
			throw new Error(
				`performance report must not contain user data field ${key}`,
			);
		}
		assertNoUserData(child);
	}
}

function assertFiniteSamples(
	value: unknown,
	field: string,
): asserts value is number[] {
	if (
		!Array.isArray(value) ||
		value.length === 0 ||
		value.some(
			(sample) => typeof sample !== "number" || !Number.isFinite(sample),
		)
	) {
		throw new Error(`${field} must contain finite samples`);
	}
}

function assertSurfaceSamples(
	value: unknown,
): asserts value is TerminalSurfacePerformanceSamples {
	assertNoUserData(value);
	if (!value || typeof value !== "object") {
		throw new Error("performance samples must be an object");
	}
	const samples = value as Record<string, unknown>;
	for (const field of REQUIRED_ARRAY_FIELDS) {
		assertFiniteSamples(samples[field], field);
	}
	for (const field of REQUIRED_NUMBER_FIELDS) {
		if (
			typeof samples[field] !== "number" ||
			!Number.isFinite(samples[field])
		) {
			throw new Error(`${field} must be a finite number`);
		}
	}
}

function assertLaunchSamples(
	value: unknown,
): asserts value is TerminalLaunchPerformanceSamples {
	assertNoUserData(value);
	if (!value || typeof value !== "object") {
		throw new Error("launch performance samples must be an object");
	}
	const samples = value as Record<string, unknown>;
	if (!samples.launchPhaseMs || typeof samples.launchPhaseMs !== "object") {
		throw new Error("launchPhaseMs is required");
	}
	const launchTotalMs = samples.launchTotalMs;
	assertFiniteSamples(launchTotalMs, "launchTotalMs");
	if (launchTotalMs.length !== 30) {
		throw new Error(
			"launch total and phases must describe the same 30 warm runs exactly",
		);
	}
	const validLaunchIdentity =
		(samples.launchSource === "deterministic-fixture" &&
			(samples.launchProvider === "fixture" ||
				samples.launchProvider === "tui-fixture")) ||
		(samples.launchSource === "provider-observation" &&
			(samples.launchProvider === "claude" ||
				samples.launchProvider === "codex"));
	if (!validLaunchIdentity) {
		throw new Error("launch source/provider combination is invalid");
	}
	const launchPhaseMs = samples.launchPhaseMs as Record<string, unknown>;
	for (const phase of TERMINAL_LAUNCH_PHASES) {
		const phaseSamples = launchPhaseMs[phase];
		assertFiniteSamples(phaseSamples, `launchPhaseMs.${phase}`);
		if (phaseSamples.length !== launchTotalMs.length) {
			throw new Error(
				"launch total and phases must describe the same 30 warm runs",
			);
		}
	}
	const typedPhases = launchPhaseMs as Record<TerminalLaunchPhase, number[]>;
	for (let index = 0; index < launchTotalMs.length; index += 1) {
		if (
			typedPhases.firstProviderByte[index] >
				typedPhases.firstXtermParsed[index] ||
			typedPhases.firstXtermParsed[index] > typedPhases.firstPaint[index] ||
			typedPhases.firstPaint[index] > launchTotalMs[index]
		) {
			throw new Error(
				`launch correlation order is invalid at warm run ${index}`,
			);
		}
		const childPhaseSum = [
			"commandIngress",
			"availabilityAndLock",
			"durableCreateCommit",
			"launchFileMaterialize",
			"checkpointLookup",
			"childEnvironment",
			"ptyOpenAndSpawn",
			"outputReaderReady",
			"firstProviderByte",
		].reduce(
			(sum, phase) => sum + typedPhases[phase as TerminalLaunchPhase][index],
			0,
		);
		if (childPhaseSum > launchTotalMs[index]) {
			throw new Error(
				`launch child phase sum exceeds total at warm run ${index}`,
			);
		}
	}
}

function distribution(samples: number[]): Distribution {
	const sorted = [...samples].sort((left, right) => left - right);
	const middle = Math.floor(sorted.length / 2);
	const median =
		sorted.length % 2 === 0
			? (sorted[middle - 1] + sorted[middle]) / 2
			: sorted[middle];
	const p95 = sorted[Math.ceil(sorted.length * 0.95) - 1];
	return {
		count: sorted.length,
		median,
		p95,
		max: sorted[sorted.length - 1],
	};
}

export function buildTerminalSurfacePerformanceReport(
	value: unknown,
): TerminalSurfacePerformanceReport {
	assertSurfaceSamples(value);
	return {
		schemaVersion: 1,
		transport: value.transport,
		inputPhaseMs:
			value.transport === "tauri-ipc"
				? (Object.fromEntries(
						TERMINAL_INPUT_TRACE_PHASES.map((phase) => [
							phase,
							distribution(
								(value.inputTraceSamples ?? []).map((sample) => sample[phase]),
							),
						]),
					) as Record<TerminalInputTracePhase, Distribution>)
				: undefined,
		fixture: { ...value.fixture },
		keyLatencyMs: distribution(value.keyLatencyMs),
		imeCommitLatencyMs: distribution(value.imeCommitLatencyMs),
		revisitLatencyMs: distribution(value.revisitLatencyMs),
		unloadedTimerDriftMs: distribution(value.unloadedTimerDriftMs),
		injectedLoadTimerDriftMs: distribution(value.injectedLoadTimerDriftMs),
		scrollLatencyMs: distribution(value.scrollLatencyMs),
		restoreLatencyMs: distribution(value.restoreLatencyMs),
		uiHeartbeatDriftMs: distribution(value.uiHeartbeatDriftMs),
		workspaceSelectionLatencyMs: distribution(
			value.workspaceSelectionLatencyMs,
		),
		rendererQueuedCodeUnits: distribution(value.rendererQueuedCodeUnits),
		rendererPeakQueuedCodeUnits: distribution(
			value.rendererPeakQueuedCodeUnits,
		),
		rendererDroppedBacklogs: value.rendererDroppedBacklogs,
		snapshotResyncs: value.snapshotResyncs,
		rendererLongStallsOver100Ms: value.rendererLongStallsOver100Ms,
		throughputMiBPerSecond: value.throughputMiBPerSecond,
	};
}

export function buildTerminalLaunchPerformanceReport(
	value: unknown,
): TerminalLaunchPerformanceReport {
	assertLaunchSamples(value);
	return {
		schemaVersion: 1,
		launchSource: value.launchSource,
		launchProvider: value.launchProvider,
		launchTotalMs: distribution(value.launchTotalMs),
		launchPhaseMs: Object.fromEntries(
			TERMINAL_LAUNCH_PHASES.map((phase) => [
				phase,
				distribution(value.launchPhaseMs[phase]),
			]),
		) as Record<TerminalLaunchPhase, Distribution>,
		launchPhaseOwnership: { ...TERMINAL_LAUNCH_PHASE_OWNERSHIP },
	};
}

function addMaximumFailure(
	failures: string[],
	label: string,
	actual: number,
	maximum: number,
): void {
	if (actual > maximum) {
		failures.push(`${label} ${actual} exceeded ${maximum}`);
	}
}

export function checkTerminalPerformanceBudgets(
	report: TerminalSurfacePerformanceReport,
): string[] {
	const failures: string[] = [];
	addMaximumFailure(
		failures,
		"median key latency",
		report.keyLatencyMs.median,
		75,
	);
	addMaximumFailure(
		failures,
		"worst key latency",
		report.keyLatencyMs.max,
		300,
	);
	addMaximumFailure(
		failures,
		"revisit latency",
		report.revisitLatencyMs.max,
		300,
	);
	addMaximumFailure(
		failures,
		"unloaded timer drift",
		report.unloadedTimerDriftMs.max,
		150,
	);
	addMaximumFailure(
		failures,
		"injected-load timer drift",
		report.injectedLoadTimerDriftMs.max,
		2500,
	);
	addMaximumFailure(
		failures,
		"scroll latency",
		report.scrollLatencyMs.max,
		150,
	);
	addMaximumFailure(
		failures,
		"restore latency",
		report.restoreLatencyMs.max,
		1000,
	);
	addMaximumFailure(
		failures,
		"UI heartbeat drift",
		report.uiHeartbeatDriftMs.max,
		150,
	);
	addMaximumFailure(
		failures,
		"workspace selection latency",
		report.workspaceSelectionLatencyMs.max,
		300,
	);
	addMaximumFailure(
		failures,
		"renderer current queue",
		report.rendererQueuedCodeUnits.max,
		2_097_152,
	);
	addMaximumFailure(
		failures,
		"renderer peak queue",
		report.rendererPeakQueuedCodeUnits.max,
		2_097_152,
	);
	addMaximumFailure(
		failures,
		"dropped backlog",
		report.rendererDroppedBacklogs,
		0,
	);
	addMaximumFailure(failures, "snapshot resync", report.snapshotResyncs, 0);
	addMaximumFailure(
		failures,
		"renderer long stall",
		report.rendererLongStallsOver100Ms,
		0,
	);
	return failures;
}

export function checkTerminalEndToEndPerformanceBudgets(
	report: TerminalSurfacePerformanceReport,
): string[] {
	if (report.transport !== "tauri-ipc") {
		return ["end-to-end performance requires tauri-ipc transport"];
	}
	return checkTerminalPerformanceBudgets(report);
}

export function formatTerminalPerformanceSummary(
	report: TerminalSurfacePerformanceReport,
): string {
	return [
		`transport: ${report.transport}`,
		`key latency median ${report.keyLatencyMs.median.toFixed(2)} ms / p95 ${report.keyLatencyMs.p95.toFixed(2)} ms / max ${report.keyLatencyMs.max.toFixed(2)} ms`,
		`renderer queue current ${report.rendererQueuedCodeUnits.max} / peak ${report.rendererPeakQueuedCodeUnits.max}`,
		`drop ${report.rendererDroppedBacklogs} / resync ${report.snapshotResyncs} / stalls ${report.rendererLongStallsOver100Ms}`,
		`throughput ${report.throughputMiBPerSecond.toFixed(2)} MiB/s`,
	].join("\n");
}

export function formatTerminalLaunchPerformanceSummary(
	report: TerminalLaunchPerformanceReport,
): string {
	return [
		`source: ${report.launchSource}`,
		`provider: ${report.launchProvider}`,
		`launch total median ${report.launchTotalMs.median.toFixed(2)} ms / p95 ${report.launchTotalMs.p95.toFixed(2)} ms / max ${report.launchTotalMs.max.toFixed(2)} ms`,
		...TERMINAL_LAUNCH_PHASES.map((phase) => {
			const sample = report.launchPhaseMs[phase];
			return `${phase} (${report.launchPhaseOwnership[phase]}): median ${sample.median.toFixed(2)} ms / p95 ${sample.p95.toFixed(2)} ms / max ${sample.max.toFixed(2)} ms`;
		}),
	].join("\n");
}
