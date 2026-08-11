import type { Terminal } from "@xterm/xterm";
import type { TerminalOutputSchedulerMetrics } from "./terminalOutputScheduler";

export type TerminalRendererPerformancePhase =
	| "frontend_request_to_command_response"
	| "channel_receive"
	| "first_xterm_parsed"
	| "first_paint";

interface TerminalPerformanceCollector {
	recordPhase(
		phase: TerminalRendererPerformancePhase,
		durationMs: number,
	): void;
	recordRendererMetrics(metrics: TerminalOutputSchedulerMetrics): void;
	recordInputPoint(
		sequence: number,
		phase: "on_data" | "channel_receive" | "xterm_parsed" | "paint",
		atUnixMs: number,
	): void;
	takeLaunchOrigin?(agentSessionId: string): number | null;
}

export function takeTerminalLaunchPerformanceOrigin(
	agentSessionId: string,
): number | null {
	if (typeof window === "undefined") return null;
	return (
		window.__RELEASH_TERMINAL_PERFORMANCE__?.takeLaunchOrigin?.(
			agentSessionId,
		) ?? null
	);
}

export function shouldReportTerminalSnapshotLaunchPhase(
	sequence: number,
): boolean {
	return Number.isSafeInteger(sequence) && sequence > 0;
}

declare global {
	interface Window {
		__RELEASH_TERMINAL_PERFORMANCE__?: TerminalPerformanceCollector;
	}
}

export function reportTerminalPerformancePhase(
	phase: TerminalRendererPerformancePhase,
	durationMs: number,
): void {
	if (typeof window === "undefined" || !Number.isFinite(durationMs)) return;
	window.__RELEASH_TERMINAL_PERFORMANCE__?.recordPhase(phase, durationMs);
}

export function isTerminalPerformanceProbeActive(): boolean {
	return (
		typeof window !== "undefined" &&
		window.__RELEASH_TERMINAL_PERFORMANCE__ !== undefined
	);
}

export function reportTerminalRendererMetrics(
	metrics: TerminalOutputSchedulerMetrics,
): void {
	if (typeof window === "undefined") return;
	window.__RELEASH_TERMINAL_PERFORMANCE__?.recordRendererMetrics({
		...metrics,
	});
}

export function reportTerminalInputPerformancePoint(
	sequence: number,
	phase: "on_data" | "channel_receive" | "xterm_parsed" | "paint",
): void {
	if (typeof window === "undefined") return;
	window.__RELEASH_TERMINAL_PERFORMANCE__?.recordInputPoint(
		sequence,
		phase,
		Date.now(),
	);
}

export interface TerminalBufferProbeSnapshot {
	text: string;
	viewportY: number;
	baseY: number;
	cursorX: number;
	cursorY: number;
}

export function readTerminalLogicalBuffer(
	terminal: Terminal,
): TerminalBufferProbeSnapshot {
	const buffer = terminal.buffer.active;
	const lines: string[] = [];
	const start = Math.max(0, buffer.length - 200);
	for (let index = start; index < buffer.length; index += 1) {
		const line = buffer.getLine(index);
		if (!line) continue;
		const text = line.translateToString(true);
		// 折返し行は論理行として前行へ連結する（includes検証を桁数非依存にする）
		if (line.isWrapped && lines.length > 0) {
			lines[lines.length - 1] += text;
		} else {
			lines.push(text);
		}
	}
	return {
		text: lines.join("\n"),
		viewportY: buffer.viewportY,
		baseY: buffer.baseY,
		cursorX: buffer.cursorX,
		cursorY: buffer.cursorY,
	};
}

declare global {
	interface Window {
		__RELEASH_TERMINAL_BUFFER_READERS__?: Record<
			string,
			() => TerminalBufferProbeSnapshot
		>;
	}
}

/**
 * WebGLレンダラはDOM rowsを生成しないため、テストが画面内容・スクロール・
 * カーソルを検証する手段としてxterm bufferの読み取りを公開する。
 * 読み取りは呼び出し時のみ実行され、hot pathへのコストはない。
 */
export function registerTerminalBufferReader(
	reader: () => TerminalBufferProbeSnapshot,
): () => void {
	if (typeof window === "undefined") return () => {};
	const key = crypto.randomUUID();
	window.__RELEASH_TERMINAL_BUFFER_READERS__ ??= {};
	const readers = window.__RELEASH_TERMINAL_BUFFER_READERS__;
	readers[key] = reader;
	return () => {
		delete readers[key];
	};
}
