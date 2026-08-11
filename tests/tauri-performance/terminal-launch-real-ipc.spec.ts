import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { summarizeRealAppLoadSamples } from "../../src/test/performance/realAppLoadReport";
import {
	buildTerminalLaunchPerformanceReport,
	formatTerminalLaunchPerformanceSummary,
	type TerminalLaunchPerformanceSamples,
	type TerminalLaunchPhase,
} from "../../src/test/performance/terminalPerformanceReport";
import { collectExecutionConditions, terminalBufferContains } from "./helpers";

type LaunchProvider = "fixture" | "tui-fixture" | "claude" | "codex";

const TUI_FIXTURE_READY_MARKER = "LAUNCH-TUI-READY";
const INTERACTIVE_PROBE = "zqzq";

interface TerminalLaunchPerformanceSample {
	phase: string;
	durationMs: number;
}

const BACKEND_PHASES: Record<string, TerminalLaunchPhase> = {
	"terminal.launch.command_ingress": "commandIngress",
	"terminal.launch.availability_and_lock": "availabilityAndLock",
	"terminal.launch.durable_create_commit": "durableCreateCommit",
	"terminal.launch.launch_file_materialize": "launchFileMaterialize",
	"terminal.launch.checkpoint_lookup": "checkpointLookup",
	"terminal.launch.child_environment": "childEnvironment",
	"terminal.launch.pty_open_and_spawn": "ptyOpenAndSpawn",
	"terminal.launch.output_reader_ready": "outputReaderReady",
	"terminal.launch.first_provider_byte": "firstProviderByte",
};

// optional phase: hook（SessionStart）は実provider起動でのみ発火し、
// fixture起動では0件になる。exactly-once assertの対象にせず、
// 存在したsampleだけをartifactへ集計する。
const OPTIONAL_BACKEND_PHASES = {
	"terminal.launch.hook_ingress": "hookIngress",
} as const;

type OptionalLaunchPhase =
	(typeof OPTIONAL_BACKEND_PHASES)[keyof typeof OPTIONAL_BACKEND_PHASES];

const launchProvider = process.env.RELEASH_PERFORMANCE_LAUNCH_PROVIDER as
	| LaunchProvider
	| undefined;
const launchDescribe = launchProvider ? describe : describe.skip;

function executableProvider(provider: LaunchProvider): "claude" | "codex" {
	return provider === "claude" ? "claude" : "codex";
}

function emptyLaunchPhases(): Record<TerminalLaunchPhase, number[]> {
	return {
		commandIngress: [],
		availabilityAndLock: [],
		durableCreateCommit: [],
		launchFileMaterialize: [],
		checkpointLookup: [],
		childEnvironment: [],
		ptyOpenAndSpawn: [],
		outputReaderReady: [],
		firstProviderByte: [],
		firstXtermParsed: [],
		firstPaint: [],
	};
}

launchDescribe("Provider AgentSession real Tauri launch performance", () => {
	if (!launchProvider) return;
	if (
		!(["fixture", "tui-fixture", "claude", "codex"] as const).includes(
			launchProvider,
		)
	) {
		throw new Error(`Unknown launch provider: ${launchProvider}`);
	}

	const provider = launchProvider;
	const worktreePath = process.cwd();
	const launchTotalMs: number[] = [];
	const launchPhaseMs = emptyLaunchPhases();
	const optionalLaunchPhaseMs: Record<OptionalLaunchPhase, number[]> = {
		hookIngress: [],
	};
	const interactiveReadyMs: number[] = [];
	const echoRoundtripMs: number[] = [];
	const measuresInteractiveReady = provider !== "fixture";

	async function measureInteractiveReady(startedAt: number): Promise<void> {
		const textarea = await $(".xterm-helper-textarea");
		if (provider === "tui-fixture") {
			await browser.waitUntil(
				async () => terminalBufferContains(TUI_FIXTURE_READY_MARKER),
				{ timeout: 30_000, interval: 20 },
			);
			interactiveReadyMs.push(
				(await browser.execute(() => performance.now())) - startedAt,
			);
			await textarea.click();
			const echoStartedAt = await browser.execute(() => performance.now());
			await textarea.addValue("p\r");
			await browser.waitUntil(async () => terminalBufferContains("echo:p"), {
				timeout: 30_000,
				interval: 20,
			});
			echoRoundtripMs.push(
				(await browser.execute(() => performance.now())) - echoStartedAt,
			);
			return;
		}
		// 実provider: 入力受付可能になるまでprobe文字列を打鍵し続け、
		// TUIがechoした時点をinteractive-readyの観測値とする（Enterは送らない）。
		await textarea.click();
		await browser.waitUntil(
			async () => {
				if (await terminalBufferContains(INTERACTIVE_PROBE)) return true;
				await textarea.addValue(INTERACTIVE_PROBE);
				return false;
			},
			{ timeout: 60_000, interval: 500 },
		);
		interactiveReadyMs.push(
			(await browser.execute(() => performance.now())) - startedAt,
		);
	}

	async function cleanup(agentSessionId: string): Promise<void> {
		await browser.execute(() => {
			window.__RELEASH_TERMINAL_PERFORMANCE_SESSION_DRIVER__?.clearSession();
		});
		await browser.pause(10);
		await browser.tauri.execute(async ({ core }, sessionId) => {
			const outcome = await core.invoke<
				"archived" | "already_archived" | "delete_confirmation_required"
			>("archive_agent_session", {
				agentSessionId: sessionId,
				callerRequestId: `performance-archive.${crypto.randomUUID()}`,
			});
			if (outcome === "delete_confirmation_required") {
				await core.invoke("confirm_agent_session_archive_delete", {
					agentSessionId: sessionId,
					callerRequestId: `performance-confirm-delete.${crypto.randomUUID()}`,
				});
				return;
			}
			await core.invoke("delete_agent_session", {
				agentSessionId: sessionId,
				callerRequestId: `performance-delete.${crypto.randomUUID()}`,
			});
		}, agentSessionId);
	}

	async function launchAndPaint(run: number, collect: boolean): Promise<void> {
		if (collect) {
			await browser.tauri.execute(({ core }) =>
				core.invoke("start_terminal_launch_performance_collection"),
			);
		}
		const before = await browser.execute(() => ({
			parsed:
				window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.phases
					.first_xterm_parsed?.length ?? 0,
			paint:
				window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.phases.first_paint
					?.length ?? 0,
			startedAt: performance.now(),
		}));
		const agentSessionId = await browser.tauri.execute(
			({ core }, request) =>
				core.invoke<string>("create_agent_session", request),
			{
				workspaceIdentity: worktreePath,
				worktreePath,
				provider: executableProvider(provider),
				rows: 24,
				cols: 80,
				callerRequestId: `performance-launch-${provider}-${run}.${crypto.randomUUID()}`,
			},
		);
		await browser.execute(
			(attachment) => {
				const driver = window.__RELEASH_TERMINAL_PERFORMANCE_SESSION_DRIVER__;
				if (!driver) throw new Error("Terminal performance Session driver is missing");
				driver.mountSession(attachment);
			},
			{
				agentSessionId,
				workspaceIdentity: worktreePath,
				worktreePath,
				provider: executableProvider(provider),
				launchStartedAt: before.startedAt,
			},
		);
		await browser.waitUntil(
			() =>
				browser.execute(
					({ parsed, paint }) => {
						const phases = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.phases;
						return (
							(phases?.first_xterm_parsed?.length ?? 0) === parsed + 1 &&
							(phases?.first_paint?.length ?? 0) === paint + 1
						);
					},
					before,
				),
			{ timeout: 30_000, interval: 20 },
		);
		const frontend = await browser.execute(({ parsed, paint, startedAt }) => {
			const phases = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.phases;
			const firstXtermParsed = phases?.first_xterm_parsed?.[parsed];
			const firstPaint = phases?.first_paint?.[paint];
			if (firstXtermParsed === undefined || firstPaint === undefined) {
				throw new Error("Correlated renderer launch phases are missing");
			}
			return {
				firstXtermParsed,
				firstPaint,
				total: performance.now() - startedAt,
			};
		}, before);

		if (collect) {
			const backend = await browser.tauri.execute(({ core }) =>
				core.invoke<TerminalLaunchPerformanceSample[]>(
					"take_terminal_launch_performance_samples",
				),
			);
			for (const [backendName, reportName] of Object.entries(BACKEND_PHASES)) {
				const samples = backend.filter((sample) => sample.phase === backendName);
				if (samples.length !== 1) {
					throw new Error(
						`warm run ${run} expected one ${backendName}, received ${samples.length}; phases=${backend.map((sample) => sample.phase).join(",")}`,
					);
				}
				launchPhaseMs[reportName].push(samples[0].durationMs);
			}
			for (const [backendName, reportName] of Object.entries(
				OPTIONAL_BACKEND_PHASES,
			)) {
				for (const sample of backend.filter(
					(sample) => sample.phase === backendName,
				)) {
					optionalLaunchPhaseMs[reportName].push(sample.durationMs);
				}
			}
			launchPhaseMs.firstXtermParsed.push(frontend.firstXtermParsed);
			launchPhaseMs.firstPaint.push(frontend.firstPaint);
			launchTotalMs.push(frontend.total);
			if (measuresInteractiveReady) {
				await measureInteractiveReady(before.startedAt);
			}
		}
		await cleanup(agentSessionId);
	}

	it(`${provider}をproduction経路で30 warm run計測しfirst paintまで相関する`, async () => {
		await launchAndPaint(-1, false);
		for (let run = 0; run < 30; run += 1) {
			await launchAndPaint(run, true);
		}

		const samples: TerminalLaunchPerformanceSamples = {
			launchSource:
				provider === "fixture" || provider === "tui-fixture"
					? "deterministic-fixture"
					: "provider-observation",
			launchProvider: provider,
			launchTotalMs,
			launchPhaseMs,
		};
		const report = buildTerminalLaunchPerformanceReport(samples);
		// optional phase（hook_ingress等）は全warm runに現れる保証がないため、
		// 30-run正規phaseのschema検証と分離して集計する（0件のphaseは省く）。
		const optionalLaunchPhases = Object.fromEntries(
			Object.entries(optionalLaunchPhaseMs)
				.filter(([, phaseSamples]) => phaseSamples.length > 0)
				.map(([phase, phaseSamples]) => [
					phase,
					summarizeRealAppLoadSamples(phaseSamples),
				]),
		);
		const optionalSummaryLines = Object.entries(optionalLaunchPhases).map(
			([phase, summary]) =>
				`${phase} (hook, optional): n=${summary.count} median ${summary.medianMs.toFixed(2)} ms / p95 ${summary.p95Ms.toFixed(2)} ms / max ${summary.maxMs.toFixed(2)} ms`,
		);
		const executionConditions = await collectExecutionConditions();
		const artifactDirectory = join(
			process.cwd(),
			"performance-results",
			"tauri-performance",
		);
		await mkdir(artifactDirectory, { recursive: true });
		await Promise.all([
			writeFile(
				join(artifactDirectory, `terminal-launch-${provider}-real.json`),
				`${JSON.stringify(
					{ ...report, optionalLaunchPhases, executionConditions },
					null,
					2,
				)}\n`,
			),
			writeFile(
				join(artifactDirectory, `terminal-launch-${provider}-real.txt`),
				`${[formatTerminalLaunchPerformanceSummary(report), ...optionalSummaryLines].join("\n")}\n`,
			),
		]);
		expect(report.launchTotalMs.count).toBe(30);
		expect(report.launchPhaseMs.firstPaint.count).toBe(30);

		if (measuresInteractiveReady) {
			const interactiveReport = {
				schemaVersion: 1,
				launchProvider: provider,
				meaning:
					provider === "tui-fixture"
						? "interactiveReady = READY marker painted; echoRoundtrip = input line to echoed output painted"
						: "interactiveReady = first probe keystroke echoed by the provider TUI (500ms probe cadence, no Enter sent)",
				interactiveReadyMs: summarizeRealAppLoadSamples(interactiveReadyMs),
				echoRoundtripMs: summarizeRealAppLoadSamples(echoRoundtripMs),
				executionConditions,
			};
			await writeFile(
				join(
					artifactDirectory,
					`terminal-launch-${provider}-interactive.json`,
				),
				`${JSON.stringify(interactiveReport, null, 2)}\n`,
			);
			expect(interactiveReport.interactiveReadyMs.count).toBe(30);
		}
	});
});
