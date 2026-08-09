import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
	buildTerminalSurfacePerformanceReport,
	checkTerminalEndToEndPerformanceBudgets,
	formatTerminalPerformanceSummary,
	type TerminalInputTraceSample,
} from "../../src/test/performance/terminalPerformanceReport";
import { terminalBufferContains } from "./helpers";

interface BackendInputSample {
	sequence: number;
	onDataToCommandIngressMs: number;
	commandIngressToAdmissionMs: number;
	admissionToWriterEnqueueMs: number;
	writerEnqueueToOutputReadMs: number;
	outputReadToModelApplyMs: number;
	modelApplyToEventPublishMs: number;
	eventPublishedAtUnixMs: number;
}

const AGENT_TUI_FRAME =
	"\u001b[38;5;220m◆ tool\u001b[0m 日本語🙂 wide\r\n" +
	"\u001b[2K\r\u001b[32m✓ completed\u001b[0m\r\n" +
	"\u001b[2A\u001b[12C\u001b[1mredraw\u001b[0m\u001b[2B\r\n" +
	"history-line 日本語🙂\r\n";
const TARGET_FIXTURE_BYTES = 10 * 1024 * 1024;

describe("Terminal Surface real Tauri IPC performance harness", () => {
	let keyLatencyMs: number[] = [];
	let inputTraceSamples: TerminalInputTraceSample[] = [];

	it("実Tauri command、実PTY、Channel、xtermを通って入力を表示する", async () => {
		const ready = await $('[data-testid="performance-terminal-ready"]');
		await ready.waitForDisplayed();
		await browser.waitUntil(async () => (await ready.getText()) === "ready");
		expect(await $$(".xterm-helper-textarea")).toHaveLength(1);

		const performanceTerminal = await $("[data-testid=performance-terminal]");
		const workspacePath = await performanceTerminal.getAttribute(
			"data-owner-workspace-path",
		);
		expect(workspacePath).toMatch(/^releash-performance-terminal-/);
		const surface = await browser.tauri.execute(({ core }, ownerWorkspacePath) =>
			core.invoke<{ session_key: string }>("get_terminal_surface", {
				owner: {
					kind: "workspace",
					workspacePath: ownerWorkspacePath,
				},
			}),
			workspacePath,
		);
		expect(surface.session_key.length).toBeGreaterThan(0);

		const terminalInput = await $(".xterm-helper-textarea");
		await terminalInput.click();
		const marker = "REAL-TAURI-IPC-MARKER";
		await terminalInput.addValue(marker);

		await browser.waitUntil(async () => terminalBufferContains(marker));
	});

	it("16入力をRust ingressからxterm paintまで匿名sequenceで相関する", async () => {
		await browser.tauri.execute(({ core }) =>
			core.invoke("start_terminal_input_performance_collection"),
		);
		await browser.execute(() => {
			const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
			if (!state) throw new Error("Terminal performance collector is missing");
			state.inputPoints = {};
		});

		const terminalInput = await $(".xterm-helper-textarea");
		await terminalInput.click();
		for (const key of "abcdefghijklmnop") {
			const previousPaintCount = await browser.execute(
				() =>
					Object.values(
						window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {},
					).filter((point) => point.paint !== undefined).length,
			);
			await terminalInput.addValue(key);
			await browser.waitUntil(async () =>
				browser.execute((count) => {
					const points =
						window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {};
					return (
						Object.values(points).filter((point) => point.paint !== undefined)
							.length > count
					);
				}, previousPaintCount),
			);
		}
		await browser.waitUntil(async () =>
			terminalBufferContains("abcdefghijklmnop"),
		);

		const backend = await browser.tauri.execute(({ core }) =>
			core.invoke<BackendInputSample[]>(
				"take_terminal_input_performance_samples",
			),
		);
		const frontend = await browser.execute(
			() => window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {},
		);

		expect(backend).toHaveLength(16);
		const correlatedTraces: TerminalInputTraceSample[] = [];
		for (const sample of backend) {
			const points = frontend[String(sample.sequence)];
			expect(points?.on_data).toBeDefined();
			expect(points?.channel_receive).toBeDefined();
			expect(points?.xterm_parsed).toBeDefined();
			expect(points?.paint).toBeDefined();
			const phases = {
				onDataToCommandIngress: sample.onDataToCommandIngressMs,
				commandIngressToAdmission: sample.commandIngressToAdmissionMs,
				admissionToWriterEnqueue: sample.admissionToWriterEnqueueMs,
				writerEnqueueToOutputRead: sample.writerEnqueueToOutputReadMs,
				outputReadToModelApply: sample.outputReadToModelApplyMs,
				modelApplyToEventPublish: sample.modelApplyToEventPublishMs,
				eventPublishToChannelReceive:
					(points?.channel_receive ?? 0) - sample.eventPublishedAtUnixMs,
				channelReceiveToXtermParsed:
					(points?.xterm_parsed ?? 0) - (points?.channel_receive ?? 0),
				xtermParsedToPaint:
					(points?.paint ?? 0) - (points?.xterm_parsed ?? 0),
			};
			for (const [phase, durationMs] of Object.entries(phases)) {
				if (!Number.isFinite(durationMs) || durationMs < 0) {
					throw new Error(
						`input sequence ${sample.sequence} has invalid ${phase}: ${durationMs}`,
					);
				}
			}
			const totalMs = (points?.paint ?? 0) - (points?.on_data ?? 0);
			expect(totalMs).toBeLessThanOrEqual(300);
			correlatedTraces.push({
				onDataToCommandIngressMs: phases.onDataToCommandIngress,
				commandIngressToAdmissionMs: phases.commandIngressToAdmission,
				admissionToWriterEnqueueMs: phases.admissionToWriterEnqueue,
				writerEnqueueToOutputReadMs: phases.writerEnqueueToOutputRead,
				outputReadToModelApplyMs: phases.outputReadToModelApply,
				modelApplyToEventPublishMs: phases.modelApplyToEventPublish,
				eventPublishToChannelReceiveMs: phases.eventPublishToChannelReceive,
				channelReceiveToXtermParsedMs: phases.channelReceiveToXtermParsed,
				xtermParsedToPaintMs: phases.xtermParsedToPaint,
				totalMs,
			});
		}
		inputTraceSamples = correlatedTraces;
		keyLatencyMs = correlatedTraces.map((sample) => sample.totalMs);
	});

	it("10MiB ANSI負荷中もUI操作とxtermをbudget内で維持する", async () => {
		const terminalInput = await $(".xterm-helper-textarea");
		await terminalInput.click();
		const measureInputPaint = async (data: string): Promise<number> => {
			const previousPaintCount = await browser.execute(
				() =>
					Object.values(
						window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {},
					).filter((point) => point.paint !== undefined).length,
			);
			await terminalInput.addValue(data);
			await browser.waitUntil(async () =>
				browser.execute((count) => {
					const points = Object.values(
						window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {},
					);
					return points.filter((point) => point.paint !== undefined).length > count;
				}, previousPaintCount),
			);
			return browser.execute(() => {
				const entries = Object.entries(
					window.__RELEASH_TERMINAL_PERFORMANCE_STATE__?.inputPoints ?? {},
				).sort(([left], [right]) => Number(right) - Number(left));
				const latest = entries[0]?.[1];
				if (latest?.on_data === undefined || latest.paint === undefined) {
					throw new Error("latest input paint sample is incomplete");
				}
				return latest.paint - latest.on_data;
			});
		};

		const imeCommitLatencyMs = [
			await measureInputPaint("日"),
			await measureInputPaint("本"),
		];
		await browser.waitUntil(async () =>
			terminalBufferContains("abcdefghijklmnop日本"),
		);
		await measureInputPaint("\r");

		await browser.execute(() => {
			const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
			if (!state) throw new Error("Terminal performance collector is missing");
			state.maxHeartbeatDriftMs = 0;
		});
		await browser.pause(100);
		const unloadedTimerDriftMs = await browser.execute(() => {
			const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
			if (!state) throw new Error("Terminal performance collector is missing");
			const drift = state.maxHeartbeatDriftMs;
			state.maxHeartbeatDriftMs = 0;
			state.rendererQueuedCodeUnits = [
				state.rendererMetrics.currentQueuedCodeUnits,
			];
			state.rendererPeakQueuedCodeUnits = [
				state.rendererMetrics.peakQueuedCodeUnits,
			];
			return drift;
		});

		const frameBytes = Buffer.byteLength(AGENT_TUI_FRAME, "utf8");
		const frameCount = Math.ceil(TARGET_FIXTURE_BYTES / frameBytes);
		const fixtureByteLength = frameBytes * frameCount;
		const fixtureMarker = "PERF-FIXTURE-COMPLETE";
		const pythonScript = [
			"import sys",
			`frame = ${JSON.stringify(AGENT_TUI_FRAME)}`,
			`payload = (frame * ${frameCount}).encode('utf-8')`,
			"sys.stdout.buffer.write(payload)",
			`sys.stdout.buffer.write(b'\\n${fixtureMarker}\\n')`,
			"sys.stdout.buffer.flush()",
		].join("\n");
		const encodedScript = Buffer.from(pythonScript, "utf8").toString("base64");
		const fixtureCommand = `python3 -c "import base64;exec(base64.b64decode('${encodedScript}'))"`;
		const fixtureStartedAt = await browser.execute(() => performance.now());
		await terminalInput.addValue(fixtureCommand);
		await terminalInput.addValue("\r");

		const workspaceSelectionLatencyMs = await browser.execute(async () => {
			const button = document.querySelector<HTMLButtonElement>(
				'[data-testid="performance-workspace-selection"]',
			);
			if (!button) throw new Error("workspace selection action is missing");
			const samples: number[] = [];
			for (let index = 0; index < 2; index += 1) {
				const previous = button.dataset.selection;
				const startedAt = performance.now();
				button.click();
				while (button.dataset.selection === previous) {
					await new Promise<void>((resolve) =>
						requestAnimationFrame(() => resolve()),
					);
				}
				samples.push(performance.now() - startedAt);
			}
			return samples;
		});

		await browser.waitUntil(async () => terminalBufferContains(fixtureMarker), {
			timeout: 120_000,
			interval: 100,
		});
		const fixtureCompletedAt = await browser.execute(() => performance.now());
		const injectedLoadTimerDriftMs = await browser.execute(() => {
			const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
			if (!state) throw new Error("Terminal performance collector is missing");
			return state.maxHeartbeatDriftMs;
		});

		const scrollLatencyMs = await browser.execute(async () => {
			const readers = Object.values(
				window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {},
			);
			const screen = document.querySelector<HTMLElement>(".xterm-screen");
			if (!screen || readers.length === 0) {
				throw new Error("xterm screen is missing");
			}
			const read = readers[0];
			const before = read().viewportY;
			const startedAt = performance.now();
			screen.dispatchEvent(
				new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: -600 }),
			);
			for (let frame = 0; frame < 60 && read().viewportY === before; frame += 1) {
				await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
			}
			if (read().viewportY === before) {
				throw new Error("xterm did not scroll");
			}
			return performance.now() - startedAt;
		});

		const visibility = await $(
			'[data-testid="performance-terminal-visibility"]',
		);
		await visibility.click();
		await browser.waitUntil(async () =>
			(await visibility.getAttribute("data-visible")) === "false",
		);
		const revisitStartedAt = await browser.execute(() => performance.now());
		await visibility.click();
		const ready = await $('[data-testid="performance-terminal-ready"]');
		await browser.waitUntil(async () => (await ready.getText()) === "ready");
		await browser.waitUntil(async () => terminalBufferContains(fixtureMarker));
		const revisitLatencyMs =
			(await browser.execute(() => performance.now())) - revisitStartedAt;

		const state = await browser.execute(
			() => window.__RELEASH_TERMINAL_PERFORMANCE_STATE__,
		);
		if (!state) throw new Error("Terminal performance collector is missing");
		const restoreLatencyMs = state.phases.first_xterm_parsed?.at(-1);
		if (restoreLatencyMs === undefined) {
			throw new Error("xterm restore parse phase is missing");
		}
		const report = buildTerminalSurfacePerformanceReport({
			transport: "tauri-ipc",
			inputTraceSamples,
			fixture: {
				kind: "agent-tui",
				byteLength: fixtureByteLength,
				containsAnsi: true,
				containsUnicode: true,
				containsWideCharacters: true,
				containsCursorRedraw: true,
			},
			keyLatencyMs,
			imeCommitLatencyMs,
			revisitLatencyMs: [revisitLatencyMs],
			unloadedTimerDriftMs: [unloadedTimerDriftMs],
			injectedLoadTimerDriftMs: [injectedLoadTimerDriftMs],
			scrollLatencyMs: [scrollLatencyMs],
			restoreLatencyMs: [restoreLatencyMs],
			uiHeartbeatDriftMs: [injectedLoadTimerDriftMs],
			workspaceSelectionLatencyMs,
			rendererQueuedCodeUnits: state.rendererQueuedCodeUnits,
			rendererPeakQueuedCodeUnits: state.rendererPeakQueuedCodeUnits,
			rendererDroppedBacklogs: state.rendererMetrics.droppedBacklogs,
			snapshotResyncs: state.rendererMetrics.snapshotResyncs,
			rendererLongStallsOver100Ms:
				state.rendererMetrics.longStallsOver100Ms,
			throughputMiBPerSecond:
				fixtureByteLength /
				(1024 * 1024) /
				((fixtureCompletedAt - fixtureStartedAt) / 1000),
		});
		const failures = checkTerminalEndToEndPerformanceBudgets(report);
		const artifactDirectory = join(
			process.cwd(),
			"performance-results",
			"tauri-performance",
		);
		await mkdir(artifactDirectory, { recursive: true });
		await Promise.all([
			writeFile(
				join(artifactDirectory, "terminal-surface-performance-real.json"),
				`${JSON.stringify(report, null, 2)}\n`,
			),
			writeFile(
				join(artifactDirectory, "terminal-surface-performance-real.txt"),
				`${formatTerminalPerformanceSummary(report)}\n`,
			),
		]);
		expect(failures).toEqual([]);
	});
});
