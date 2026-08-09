import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { TerminalPerformanceSwitches } from "../../src/lib/terminalPerformanceSwitches";
import {
	buildRealAppLoadReport,
	formatRealAppLoadSummary,
} from "../../src/test/performance/realAppLoadReport";
import {
	collectExecutionConditions,
	type EchoSampler,
	terminalBufferContains,
	terminalBufferLinesContaining,
	type WorkspaceSelectionProbe,
} from "./helpers";

const AGENT_TUI_LINE =
	"\u001b[38;5;220m◆ tool\u001b[0m 日本語🙂 wide 0123456789 abcdefghij\r\n" +
	"\u001b[2K\r\u001b[32m✓ completed\u001b[0m step ##\r\n";
const FRAME_REPEAT = Number(
	process.env.RELEASH_PERFORMANCE_LOAD_FRAME_REPEAT ?? 8,
);
const FRAME_INTERVAL_MS = Number(
	process.env.RELEASH_PERFORMANCE_LOAD_FRAME_INTERVAL_MS ?? 16,
);
const LOAD_DURATION_MS = 15_000;
const DSR_INTERVAL_MS = 1_000;
const TYPED_KEYS = "QWXZVKUYQWXZVKUY";
// 既定300msは1打鍵ずつechoが返る逐次計測。echoレイテンシより短い値を
// 指定すると複数入力の同時in-flightを再現できる（samplerは複数pending対応）。
const TYPED_KEY_INTERVAL_MS = Number(
	process.env.RELEASH_PERFORMANCE_TYPED_KEY_INTERVAL_MS ?? 300,
);
const LOAD_COMPLETE_MARKER = "REAL-APP-LOAD-COMPLETE";

function buildSustainedFixtureCommand(): { command: string; frameBytes: number } {
	const frame = AGENT_TUI_LINE.repeat(FRAME_REPEAT);
	const frameBytes = Buffer.byteLength(frame, "utf8");
	const totalFrames = Math.ceil(LOAD_DURATION_MS / FRAME_INTERVAL_MS);
	const dsrEveryFrames = Math.max(1, Math.round(DSR_INTERVAL_MS / FRAME_INTERVAL_MS));
	// 実TUIと同じく、fixture自身がstdinを読み「input:」行として毎フレーム
	// 再描画する（cbreakでkernel echoは無効＝入力の画面反映はTUIフレーム経由のみ）
	// 注意: pty子プロセスはfd0/1が同一file descriptionを共有するため、
	// stdinへのO_NONBLOCKはstdoutも非blockingにしてフレーム書込を壊す。
	// selectで可読性を確認してからraw readする（stdoutはblockingのまま）。
	const pythonScript = [
		"import sys, time, tty, os, select",
		"fd = sys.stdin.fileno()",
		"tty.setcbreak(fd)",
		`frame = ${JSON.stringify(frame)}`,
		`total = ${totalFrames}`,
		`interval = ${FRAME_INTERVAL_MS} / 1000.0`,
		`dsr_every = ${dsrEveryFrames}`,
		"typed = ''",
		"started = time.monotonic()",
		"for index in range(total):",
		"    while select.select([fd], [], [], 0)[0]:",
		"        chunk = os.read(fd, 4096)",
		"        if not chunk:",
		"            break",
		"        typed += chunk.decode('utf-8', 'replace')",
		`    payload = frame + '\\x1b[2Kinput:' + typed + '\\r\\x1b[${FRAME_REPEAT * 2}A'`,
		"    if index % dsr_every == 0:",
		"        payload += '\\x1b[6n'",
		"    sys.stdout.write(payload)",
		"    sys.stdout.flush()",
		"    next_at = started + (index + 1) * interval",
		"    delay = next_at - time.monotonic()",
		"    if delay > 0:",
		"        time.sleep(delay)",
		`sys.stdout.write('\\x1b[${FRAME_REPEAT * 2 + 1}B\\r\\n${LOAD_COMPLETE_MARKER}\\r\\n')`,
		"sys.stdout.flush()",
	].join("\n");
	const encodedScript = Buffer.from(pythonScript, "utf8").toString("base64");
	return {
		command: `python3 -c "import base64;exec(base64.b64decode('${encodedScript}'))"`,
		frameBytes,
	};
}

async function terminalTextarea() {
	const selector =
		'[data-testid="right-bottom-content"] .xterm-helper-textarea';
	const element = await $(selector);
	await element.waitForExist({ timeout: 30_000 });
	return element;
}

describe("Real WorkbenchApp terminal load harness", () => {
	it("実WorkbenchAppが起動しworktree一覧から選択できる", async () => {
		const realApp = await browser.tauri.execute(({ core }) =>
			core.invoke<boolean>("get_performance_real_app_mode"),
		);
		expect(realApp).toBe(true);

		const collectorInstalled = await browser.execute(
			() => window.__RELEASH_TERMINAL_PERFORMANCE_STATE__ !== undefined,
		);
		expect(collectorInstalled).toBe(true);

		const worktreeItem = await $('[data-testid^="worktree-item-"]');
		await worktreeItem.waitForDisplayed({ timeout: 60_000 });
		await worktreeItem.click();

		const contentRegion = await $('[data-testid="main-layout-content-region"]');
		await contentRegion.waitForDisplayed({ timeout: 30_000 });

		const textarea = await terminalTextarea();
		await browser.execute((element) => {
			(element as HTMLTextAreaElement).focus();
		}, textarea);
	});

	it("持続TUI出力中に連続タイプ・IME composition・DSR・実Workspace選択を計測する", async () => {
		const textarea = await terminalTextarea();
		await browser.execute((element) => {
			(element as HTMLTextAreaElement).focus();
		}, textarea);

		await browser.execute(() => {
			const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
			if (!state) throw new Error("Terminal performance collector is missing");
			state.inputPoints = {};
			state.maxHeartbeatDriftMs = 0;
			state.longtasks = { count: 0, totalMs: 0, maxMs: 0 };
			state.rendererPeakQueuedCodeUnits = [
				state.rendererMetrics.peakQueuedCodeUnits,
			];
			// echo可視化サンプラ: armされたmarkerがterminal bufferに現れた
			// 時刻をrAFごとに監視して記録する（次paintバッチ近似ではなく、
			// 自分のechoが描画バッファに到達した実時刻を計測する）
			const sampler: EchoSampler = {
				pending: [],
				results: [],
				stopped: false,
			};
			window.__RELEASH_ECHO_SAMPLER__ = sampler;
			const tick = () => {
				if (sampler.stopped) return;
				if (sampler.pending.length > 0) {
					const readers = Object.values(
						window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {},
					);
					const texts: string[] = [];
					let latestBaseY: number | null = null;
					for (const read of readers) {
						try {
							const snapshot = read();
							latestBaseY = snapshot.baseY;
							texts.push(snapshot.text);
						} catch {
							// reader消滅は無視
						}
					}
					const seenAtUnixMs = Date.now();
					const stillPending: typeof sampler.pending = [];
					for (const pending of sampler.pending) {
						pending.framesWhileArmed += 1;
						if (texts.some((text) => text.includes(pending.marker))) {
							sampler.results.push({
								marker: pending.marker,
								seenAtUnixMs,
								armedAtUnixMs: pending.armedAtUnixMs,
								armedBaseY: pending.armedBaseY,
								seenBaseY: latestBaseY,
								framesWhileArmed: pending.framesWhileArmed,
							});
						} else {
							stillPending.push(pending);
						}
					}
					sampler.pending = stillPending;
				}
				requestAnimationFrame(tick);
			};
			requestAnimationFrame(tick);
		});

		const { command, frameBytes } = buildSustainedFixtureCommand();
		await textarea.addValue(command);
		await textarea.addValue("\r");
		// fixtureの初回フレームが流れ始めるのを待つ
		await browser.pause(500);

		// --- 負荷中の連続タイプ（echo markerの可視化時刻で計測） ---
		await browser.tauri.execute(({ core }) =>
			core.invoke("start_terminal_input_performance_collection"),
		);
		let typedSoFar = "";
		for (const key of TYPED_KEYS) {
			typedSoFar += key;
			const marker = `input:${typedSoFar}`;
			await browser.execute((marker) => {
				const sampler = window.__RELEASH_ECHO_SAMPLER__;
				if (!sampler) throw new Error("echo sampler is missing");
				const readers = Object.values(
					window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {},
				);
				let baseY: number | null = null;
				for (const read of readers) {
					try {
						baseY = read().baseY;
					} catch {
						// ignore
					}
				}
				sampler.pending.push({
					marker,
					armedAtUnixMs: Date.now(),
					armedBaseY: baseY,
					framesWhileArmed: 0,
				});
			}, marker);
			await textarea.addValue(key);
			await browser.pause(TYPED_KEY_INTERVAL_MS);
		}

		// --- 負荷中のIME composition（実compositionイベント列＋echo marker計測） ---
		const imeCommitText = "ワサビ";
		const imeDispatchedAt = await browser.execute((commitText) => {
			const element = document.querySelector<HTMLTextAreaElement>(
				'[data-testid="right-bottom-content"] .xterm-helper-textarea',
			);
			if (!element) throw new Error("terminal textarea is missing");
			const sampler = window.__RELEASH_ECHO_SAMPLER__;
			if (!sampler) throw new Error("echo sampler is missing");
			element.focus();
			element.dispatchEvent(
				new CompositionEvent("compositionstart", { data: "" }),
			);
			element.value += commitText;
			element.dispatchEvent(
				new CompositionEvent("compositionupdate", { data: commitText }),
			);
			sampler.pending.push({
				marker: commitText,
				armedAtUnixMs: Date.now(),
				armedBaseY: null,
				framesWhileArmed: 0,
			});
			const dispatchedAt = Date.now();
			element.dispatchEvent(
				new CompositionEvent("compositionend", { data: commitText }),
			);
			return dispatchedAt;
		}, imeCommitText);
		await browser.pause(500);

		// --- 負荷中の実Workspace選択（実リスト再取得・remountを伴う） ---
		const workspaceSelectionLatencyMs: number[] = [];
		// 並び替え・再renderに影響されないよう、行のtestidを先に確定し
		// 交互選択（別worktree→元worktree）を正確なselectorで行う
		const worktreeItemIds = await browser.execute(() =>
			Array.from(
				document.querySelectorAll('[data-testid^="worktree-item-"]'),
			).map((element) => element.getAttribute("data-testid") ?? ""),
		);
		const selectionTargetIds =
			worktreeItemIds.length >= 2
				? [worktreeItemIds[1], worktreeItemIds[0]]
				: [worktreeItemIds[0], worktreeItemIds[0]];
		const selectionSplits: Array<{
			bodyFirstMs: number | null;
			contentFirstMs: number | null;
		}> = [];
		for (const targetId of selectionTargetIds) {
			// click前にobserverを設置して初回mutationを取り逃がさない
			await browser.execute(() => {
				const region = document.querySelector(
					'[data-testid="main-layout-content-region"]',
				);
				if (!region) throw new Error("content region is missing");
				const probe: WorkspaceSelectionProbe = {
					startedAtUnixMs: 0,
					bodyFirstUnixMs: null,
					contentFirstUnixMs: null,
				};
				const bodyObserver = new MutationObserver(() => {
					if (probe.bodyFirstUnixMs === null) {
						probe.bodyFirstUnixMs = Date.now();
					}
				});
				const contentObserver = new MutationObserver(() => {
					if (probe.contentFirstUnixMs === null) {
						probe.contentFirstUnixMs = Date.now();
						bodyObserver.disconnect();
						contentObserver.disconnect();
					}
				});
				bodyObserver.observe(document.body, {
					childList: true,
					subtree: true,
					attributes: true,
				});
				contentObserver.observe(region, {
					childList: true,
					subtree: true,
					attributes: true,
				});
				probe.startedAtUnixMs = Date.now();
				window.__RELEASH_SELECTION_PROBE__ = probe;
			});
			const target = await $(`[data-testid="${targetId}"]`);
			await target.click();
			await browser.waitUntil(
				() =>
					browser.execute(
						() => window.__RELEASH_SELECTION_PROBE__?.contentFirstUnixMs != null,
					),
				{ timeout: 8_000, timeoutMsg: "content region never mutated" },
			).catch(() => {});
			const probe = await browser.execute(() => {
				const value = window.__RELEASH_SELECTION_PROBE__;
				if (!value) throw new Error("selection probe is missing");
				return value;
			});
			selectionSplits.push({
				bodyFirstMs:
					probe.bodyFirstUnixMs === null
						? null
						: probe.bodyFirstUnixMs - probe.startedAtUnixMs,
				contentFirstMs:
					probe.contentFirstUnixMs === null
						? null
						: probe.contentFirstUnixMs - probe.startedAtUnixMs,
			});
			// 単一worktree環境では2回目（同一対象への再クリック）は状態無変化の
			// no-opになり得るため、成立した選択のみを記録する
			if (probe.contentFirstUnixMs !== null) {
				workspaceSelectionLatencyMs.push(
					probe.contentFirstUnixMs - probe.startedAtUnixMs,
				);
			}
		}

		// --- fixture完了待ちとDSR回収窓 ---
		await browser.waitUntil(
			async () => terminalBufferContains(LOAD_COMPLETE_MARKER),
			{ timeout: 90_000, interval: 250 },
		);

		// IME確定文字列のPTY二重送信は累積echo行内の2回出現として観測される
		// （redraw・scrollback由来の行重複は送信のexactly-onceと矛盾しない）
		const imeEchoLines = await terminalBufferLinesContaining(imeCommitText);
		expect(imeEchoLines.length).toBeGreaterThanOrEqual(1);
		const doubledImeEchoLines = imeEchoLines.filter(
			(line) => line.split(imeCommitText).length - 1 > 1,
		);
		expect(doubledImeEchoLines).toEqual([]);

		// 描画結果の目視検証用スクリーンショット（WebGL描画崩壊の検出手段）
		const screenshotDirectory = join(
			process.cwd(),
			"performance-results",
			"tauri-performance",
		);
		await mkdir(screenshotDirectory, { recursive: true });
		const terminalRegion = await $('[data-testid="right-bottom-content"]');
		await terminalRegion.saveScreenshot(
			join(screenshotDirectory, "terminal-real-app-load-screenshot.png"),
		);

		// WebGL addonが実際に有効化されたか（fallbackしていないか）の判定材料
		const webglCanvasDetected = await browser.execute(() => {
			const region = document.querySelector(
				'[data-testid="right-bottom-content"]',
			);
			return Boolean(region?.querySelector("canvas"));
		});

		const state = await browser.execute(
			() => window.__RELEASH_TERMINAL_PERFORMANCE_STATE__,
		);
		if (!state) throw new Error("Terminal performance collector is missing");

		// echoサンプラを停止し結果を回収
		const echoResults = await browser.execute(() => {
			const sampler = window.__RELEASH_ECHO_SAMPLER__;
			if (!sampler) throw new Error("echo sampler is missing");
			sampler.stopped = true;
			return sampler.results;
		});
		console.log(
			"[echo diagnostics] baseY advance while armed:",
			JSON.stringify(
				echoResults.map((result) => ({
					marker: result.marker.slice(-4),
					frames: result.framesWhileArmed,
					baseYDelta:
						result.armedBaseY === null || result.seenBaseY === null
							? null
							: result.seenBaseY - result.armedBaseY,
				})),
			),
		);

		// typed key: DSRのCPR応答もinputPointsへ混入するため、indexでなく
		// 「armedAt直後のon_data」で対応するentryをjoinする（時刻窓join）
		const inputPoints = Object.entries(state.inputPoints)
			.map(([sequence, points]) => ({ sequence: Number(sequence), points }))
			.sort((left, right) => left.sequence - right.sequence);
		const typedEchoResults = echoResults.filter(
			(result) => result.marker !== imeCommitText,
		);
		expect(typedEchoResults.length).toBeGreaterThanOrEqual(
			TYPED_KEYS.length / 2,
		);
		const matchTypedEntry = (result: (typeof typedEchoResults)[number]) =>
			inputPoints.find(
				(entry) =>
					entry.points.on_data !== undefined &&
					entry.points.on_data >= result.armedAtUnixMs &&
					entry.points.on_data <= result.seenAtUnixMs,
			);
		const typedKeyLatencyMs = typedEchoResults
			.map((result) => {
				const onData = matchTypedEntry(result)?.points.on_data;
				return onData === undefined ? null : result.seenAtUnixMs - onData;
			})
			.filter((value): value is number => value !== null && value >= 0);

		// backend区間: on_data→event publish（sequence厳密join）と
		// publish→echo可視（配送＋parseの合算）に分解する
		const backendSamples = await browser.tauri.execute(({ core }) =>
			core.invoke<Array<{ sequence: number; eventPublishedAtUnixMs: number }>>(
				"take_terminal_input_performance_samples",
			),
		);
		const backendBySequence = new Map(
			backendSamples.map((sample) => [sample.sequence, sample]),
		);
		const typedKeyOnDataToPublishMs: number[] = [];
		const typedKeyPublishToEchoVisibleMs: number[] = [];
		for (const result of typedEchoResults) {
			const entry = matchTypedEntry(result);
			if (!entry || entry.points.on_data === undefined) continue;
			const backend = backendBySequence.get(entry.sequence);
			if (!backend) continue;
			typedKeyOnDataToPublishMs.push(
				backend.eventPublishedAtUnixMs - entry.points.on_data,
			);
			typedKeyPublishToEchoVisibleMs.push(
				result.seenAtUnixMs - backend.eventPublishedAtUnixMs,
			);
		}

		// IME commit: compositionend送出から確定文字列のecho可視化まで
		const imeEcho = echoResults.find(
			(result) => result.marker === imeCommitText,
		);
		const imeCommitLatencyMs =
			imeEcho === undefined ? [] : [imeEcho.seenAtUnixMs - imeDispatchedAt];

		// DSR CPR応答: キー入力もcompositionも発生していない時間帯のonData到着列
		const dsrReplyOnDataUnixMs = inputPoints
			.filter(
				(entry) =>
					entry.points.on_data !== undefined &&
					entry.points.on_data > imeDispatchedAt + 2_000,
			)
			.map((entry) => entry.points.on_data ?? 0);

		const loadTimerDriftMs = state.maxHeartbeatDriftMs;
		const switches = await browser.tauri.execute(({ core }) =>
			core.invoke<TerminalPerformanceSwitches>(
				"get_terminal_performance_switches",
			),
		);

		const report = buildRealAppLoadReport({
			transport: switches.disableTerminalWebsocket ? "tauri-ipc" : "websocket",
			realApp: true,
			switches: { ...switches },
			loadFixture: {
				kind: "sustained-agent-tui",
				frameBytes,
				frameIntervalMs: FRAME_INTERVAL_MS,
				durationMs: LOAD_DURATION_MS,
				dsrIntervalMs: DSR_INTERVAL_MS,
			},
			typedKeyLatencyMs,
			typedKeyOnDataToPublishMs,
			typedKeyPublishToEchoVisibleMs,
			imeCommitLatencyMs,
			dsrReplyOnDataUnixMs,
			workspaceSelectionLatencyMs,
			workspaceSelectionSplits: selectionSplits,
			loadTimerDriftMs,
			longtasks: { ...state.longtasks },
			longtasksUnsupported: state.longtasksUnsupported,
			rendererPeakQueuedCodeUnits: Math.max(
				...state.rendererPeakQueuedCodeUnits,
				0,
			),
			rendererLongStallsOver100Ms: state.rendererMetrics.longStallsOver100Ms,
			rendererDroppedBacklogs: state.rendererMetrics.droppedBacklogs,
			snapshotResyncs: state.rendererMetrics.snapshotResyncs,
			caveats: [
				"typed-key/IME latency = time until the key's own echo is visible in the terminal buffer, sampled per animation frame (frame-granularity, renderer-independent)",
				`typed key cadence = ${TYPED_KEY_INTERVAL_MS}ms (RELEASH_PERFORMANCE_TYPED_KEY_INTERVAL_MS); intervals shorter than the echo latency keep multiple keystrokes in-flight (multi-pending echo sampler)`,
				"IME commit drives the real composition event sequence (compositionstart/update/end) but the events are synthesized without the OS IME; native IME timing may differ",
				"DSR replies are xterm CPR responses to ESC[6n embedded in the load stream; cadence deviation measures pipeline queueing, not absolute latency",
				"transport is derived from the disableTerminalWebsocket switch; a channel fallback after a websocket connection failure is not distinguished in this report",
				`renderer: webgl-default=${!switches.disableWebglRenderer}; canvas-detected=${webglCanvasDetected}`,
			],
		});

		const executionConditions = await collectExecutionConditions();
		const artifactDirectory = join(
			process.cwd(),
			"performance-results",
			"tauri-performance",
		);
		await mkdir(artifactDirectory, { recursive: true });
		await Promise.all([
			writeFile(
				join(artifactDirectory, "terminal-real-app-load.json"),
				`${JSON.stringify({ ...report, executionConditions }, null, 2)}\n`,
			),
			writeFile(
				join(artifactDirectory, "terminal-real-app-load.txt"),
				`${formatRealAppLoadSummary(report)}\n`,
			),
		]);

		// Phase 1はbaseline取得が目的のため、budget合否ではなく計測成立だけを検証する
		expect(typedKeyLatencyMs.length).toBeGreaterThan(0);
		expect(imeCommitLatencyMs.length).toBeGreaterThan(0);
		expect(workspaceSelectionLatencyMs.length).toBeGreaterThanOrEqual(1);
		expect(dsrReplyOnDataUnixMs.length).toBeGreaterThan(2);
	});
});
