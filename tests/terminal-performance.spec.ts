import { expect, test, type Page } from "@playwright/test";
import { writeFile } from "node:fs/promises";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

interface PerformanceState {
	fixtureStartedAt: number;
	fixtureCompletedAt: number;
	fixtureByteLength: number;
	maxHeartbeatDriftMs: number;
	rendererMetrics: {
		currentQueuedCodeUnits: number;
		peakQueuedCodeUnits: number;
		writeCount: number;
		longStallsOver100Ms: number;
		droppedBacklogs: number;
		snapshotResyncs: number;
	};
	rendererQueuedCodeUnits: number[];
	rendererPeakQueuedCodeUnits: number[];
	phases: Record<string, number[]>;
}

async function now(page: Page): Promise<number> {
	return page.evaluate(() => performance.now());
}

function median(samples: number[]): number {
	const sorted = [...samples].sort((left, right) => left - right);
	const middle = Math.floor(sorted.length / 2);
	return sorted.length % 2 === 0
		? (sorted[middle - 1] + sorted[middle]) / 2
		: sorted[middle];
}

test("10MiB agent-TUI負荷でTerminal Surfaceのstrict performance budgetを守る", async ({
	page,
}, testInfo) => {
	test.setTimeout(120_000);
	await page.addInitScript(() => {
		const state: PerformanceState = {
			fixtureStartedAt: 0,
			fixtureCompletedAt: 0,
			fixtureByteLength: 0,
			maxHeartbeatDriftMs: 0,
			rendererMetrics: {
				currentQueuedCodeUnits: 0,
				peakQueuedCodeUnits: 0,
				writeCount: 0,
				longStallsOver100Ms: 0,
				droppedBacklogs: 0,
				snapshotResyncs: 0,
			},
			rendererQueuedCodeUnits: [],
			rendererPeakQueuedCodeUnits: [],
			phases: {},
		};
		Object.assign(window, { __RELEASH_TERMINAL_PERFORMANCE_STATE__: state });
		window.__RELEASH_TERMINAL_PERFORMANCE__ = {
			recordInputPoint: () => {},
			recordPhase: (phase, durationMs) => {
				(state.phases[phase] ??= []).push(durationMs);
			},
			recordRendererMetrics: (metrics) => {
				state.rendererMetrics = metrics;
				state.rendererQueuedCodeUnits.push(metrics.currentQueuedCodeUnits);
				state.rendererPeakQueuedCodeUnits.push(metrics.peakQueuedCodeUnits);
			},
		};
		let expected = performance.now() + 10;
		setInterval(() => {
			const current = performance.now();
			state.maxHeartbeatDriftMs = Math.max(
				state.maxHeartbeatDriftMs,
				current - expected,
			);
			expected = current + 10;
		}, 10);
	});
	await setupTauriMock(
		page,
		buildMockConfig({
			list_worktrees: [
				{
					name: "repo",
					path: "/test/repo",
					branch: "main",
					is_main: true,
					is_locked: false,
					dirty_count: 0,
					base_branch: null,
					management_kind: "working_area",
				},
			],
			list_branches_with_status: [
				{
					name: "main",
					is_default: true,
					worktree_path: "/test/repo",
					management_kind: "working_area",
					dirty_count: 0,
					is_merged: false,
					has_pr: false,
					pr_number: null,
					pr_url: null,
					ahead: 0,
					behind: 0,
					has_upstream: true,
					base_ahead: 0,
				},
			],
			attach_terminal_surface: {
				__mockTerminalPerformanceAttachment: {
					targetBytes: 10 * 1024 * 1024,
					chunkCodeUnits: 16 * 1024,
					initialReplay: "\u001bcRESTORE-MARKER ",
				},
			},
		}),
	);
	await waitForApp(page);
	expect(await page.locator('script[src="/@vite/client"]').count()).toBe(0);
	const rows = page.locator(".xterm-rows");
	await expect(rows).toContainText("RESTORE-MARKER");

	await page.waitForTimeout(100);
	const unloadedTimerDriftMs = await page.evaluate(() => {
		const state = (
			window as typeof window & {
				__RELEASH_TERMINAL_PERFORMANCE_STATE__: PerformanceState;
			}
		).__RELEASH_TERMINAL_PERFORMANCE_STATE__;
		const drift = state.maxHeartbeatDriftMs;
		state.maxHeartbeatDriftMs = 0;
		return drift;
	});

	const terminalInput = page.locator(".xterm-helper-textarea");
	await terminalInput.focus();
	const keyLatencyMs: number[] = [];
	let expectedInput = "";
	for (const key of "abcdefghijklmnop") {
		const startedAt = await now(page);
		await page.keyboard.insertText(key);
		expectedInput += key;
		await page.waitForFunction(
			(expected) =>
				document.querySelector(".xterm-rows")?.textContent?.includes(expected) ??
				false,
			expectedInput,
			{ timeout: 5_000 },
		);
		keyLatencyMs.push((await now(page)) - startedAt);
	}
	const imeCommitLatencyMs: number[] = [];
	for (const commit of ["日", "本"]) {
		const startedAt = await now(page);
		await page.keyboard.insertText(commit);
		expectedInput += commit;
		await page.waitForFunction(
			(expected) =>
				document.querySelector(".xterm-rows")?.textContent?.includes(expected) ??
				false,
			expectedInput,
			{ timeout: 5_000 },
		);
		imeCommitLatencyMs.push((await now(page)) - startedAt);
	}
	await page.evaluate(() =>
		window.__TAURI_INTERNALS__?.invoke("start_terminal_performance_fixture"),
	);
	const workspaceSelectionLatencyMs = await page.evaluate(async () => {
		const worktree = document.querySelector<HTMLButtonElement>(
			'[data-testid="worktree-item-main"]',
		);
		if (!worktree) throw new Error("performance worktree action is missing");
		const samples: number[] = [];
		for (const expanded of ["false", "true"]) {
			const startedAt = performance.now();
			worktree.click();
			while (worktree.getAttribute("aria-expanded") !== expanded) {
				await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
			}
			samples.push(performance.now() - startedAt);
		}
		return samples;
	});
	await expect(rows).toContainText("PERF-FIXTURE-COMPLETE", {
		timeout: 30_000,
	});
	const fixtureCompletedAt = await now(page);
	const injectedLoadTimerDriftMs = await page.evaluate((completedAt) => {
		const state = (
			window as typeof window & {
				__RELEASH_TERMINAL_PERFORMANCE_STATE__: PerformanceState;
			}
		).__RELEASH_TERMINAL_PERFORMANCE_STATE__;
		state.fixtureCompletedAt = completedAt;
		return state.maxHeartbeatDriftMs;
	}, fixtureCompletedAt);
	await page.evaluate(() => {
		const scrollFixture = Array.from(
			{ length: 300 },
			(_, index) => `scroll-row-${index}\r\n`,
		).join("");
		return window.__TAURI_INTERNALS__?.invoke("write_terminal_surface", {
			data: scrollFixture,
		});
	});
	await expect(rows).toContainText("scroll-row-299");
	const rowsBeforeScroll = await rows.textContent();
	const scrollStartedAt = await now(page);
	const terminalBox = await page.locator(".xterm-screen").boundingBox();
	if (!terminalBox) throw new Error("xterm screen is not visible");
	await page.mouse.move(
		terminalBox.x + terminalBox.width / 2,
		terminalBox.y + terminalBox.height / 2,
	);
	await page.mouse.wheel(0, -600);
	await expect
		.poll(() => rows.textContent())
		.not.toBe(rowsBeforeScroll);
	const scrollLatencyMs = (await now(page)) - scrollStartedAt;
	const rowsAfterScroll = await rows.textContent();

	await page.getByRole("button", { name: "Collapse panel" }).click();
	await expect(page.getByRole("button", { name: "Expand panel" })).toBeVisible();
	const revisitStartedAt = await now(page);
	await page.getByRole("button", { name: "Expand panel" }).click();
	await expect(page.getByRole("button", { name: "Collapse panel" })).toBeVisible();
	await expect.poll(() => rows.textContent()).toBe(rowsAfterScroll);
	const revisitLatencyMs = (await now(page)) - revisitStartedAt;

	const state = await page.evaluate(
		() =>
			(
				window as typeof window & {
					__RELEASH_TERMINAL_PERFORMANCE_STATE__: PerformanceState;
				}
			).__RELEASH_TERMINAL_PERFORMANCE_STATE__,
	);
	const restoreLatencyMs = state.phases.first_xterm_parsed?.[0];
	const firstPaintMs = state.phases.first_paint?.[0];
	if (restoreLatencyMs === undefined || firstPaintMs === undefined) {
		throw new Error("xterm parse and paint performance phases are required");
	}
	const report = {
		schemaVersion: 1,
		transport: "mocked-channel",
		fixture: {
			kind: "agent-tui",
			byteLength: state.fixtureByteLength,
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
			state.fixtureByteLength /
			(1024 * 1024) /
			((state.fixtureCompletedAt - state.fixtureStartedAt) / 1000),
		firstPaintMs,
	};
	const reportJson = `${JSON.stringify(report, null, 2)}\n`;
	const reportPath = testInfo.outputPath("terminal-performance.json");
	const summary = [
		`key latency median ${median(report.keyLatencyMs).toFixed(2)} ms / max ${Math.max(...report.keyLatencyMs).toFixed(2)} ms`,
		`throughput ${report.throughputMiBPerSecond.toFixed(2)} MiB/s`,
		`queue peak ${Math.max(...report.rendererPeakQueuedCodeUnits)} / drop ${report.rendererDroppedBacklogs} / resync ${report.snapshotResyncs} / stalls ${report.rendererLongStallsOver100Ms}`,
		`UI heartbeat ${report.uiHeartbeatDriftMs[0].toFixed(2)} ms / workspace worst ${Math.max(...report.workspaceSelectionLatencyMs).toFixed(2)} ms`,
	].join("\n");
	const summaryPath = testInfo.outputPath("terminal-performance-summary.txt");
	await Promise.all([
		writeFile(reportPath, reportJson),
		writeFile(summaryPath, `${summary}\n`),
	]);
	await testInfo.attach("terminal-performance.json", {
		path: reportPath,
		contentType: "application/json",
	});
	await testInfo.attach("terminal-performance-summary.txt", {
		path: summaryPath,
		contentType: "text/plain",
	});
	console.info(summary);

	expect(report.fixture.byteLength).toBeGreaterThanOrEqual(10 * 1024 * 1024);
	expect(report.keyLatencyMs).toHaveLength(16);
	expect(median(report.keyLatencyMs)).toBeLessThanOrEqual(75);
	expect(Math.max(...report.keyLatencyMs)).toBeLessThanOrEqual(300);
	expect(Math.max(...report.imeCommitLatencyMs)).toBeLessThanOrEqual(300);
	expect(report.revisitLatencyMs[0]).toBeLessThanOrEqual(300);
	expect(report.unloadedTimerDriftMs[0]).toBeLessThanOrEqual(150);
	expect(report.injectedLoadTimerDriftMs[0]).toBeLessThanOrEqual(2500);
	expect(report.scrollLatencyMs[0]).toBeLessThanOrEqual(150);
	expect(report.restoreLatencyMs[0]).toBeLessThanOrEqual(1000);
	expect(report.uiHeartbeatDriftMs[0]).toBeLessThanOrEqual(150);
	expect(Math.max(...report.workspaceSelectionLatencyMs)).toBeLessThanOrEqual(
		300,
	);
	expect(Math.max(...report.rendererQueuedCodeUnits)).toBeLessThanOrEqual(
		2 * 1024 * 1024,
	);
	expect(Math.max(...report.rendererPeakQueuedCodeUnits)).toBeLessThanOrEqual(
		2 * 1024 * 1024,
	);
	expect(report.rendererDroppedBacklogs).toBe(0);
	expect(report.snapshotResyncs).toBe(0);
	expect(report.rendererLongStallsOver100Ms).toBe(0);
});
