import type { Terminal as XtermTerminal } from "@xterm/xterm";
import { expect, it, vi } from "vitest";
import checkpointFixture from "../../tests/fixtures/terminal-surface-checkpoint-v1.json";
import {
	applyTerminalStreamItem,
	type TerminalStreamApplyContext,
} from "./terminalSurfaceStream";

it("backend checkpointを実xtermへalternate screen・属性・wide文字・cursor込みで投影する", async () => {
	const { Terminal } =
		await vi.importActual<typeof import("@xterm/xterm")>("@xterm/xterm");
	const terminal: XtermTerminal = new Terminal({
		cols: 80,
		rows: 24,
		allowProposedApi: true,
	});
	const ctx: TerminalStreamApplyContext = {
		isCurrent: () => true,
		drainLiveOutput: async () => {},
		resizeTerminal: (cols, rows) => terminal.resize(cols, rows),
		writeToTerminal: (data) =>
			new Promise<void>((resolve) => terminal.write(data, resolve)),
		applySnapshotIdentity: vi.fn(),
		syncPtySizeAfterEmptySnapshot: vi.fn(),
		completeRecovery: vi.fn(),
		setRunning: vi.fn(),
		completeInitialSnapshot: vi.fn(),
		flushStartupInput: vi.fn(),
		enqueueOutput: vi.fn(),
		setProcessedReportUnits: vi.fn(),
		reportProcessed: vi.fn(),
	};
	try {
		await applyTerminalStreamItem(
			{
				type: "snapshot",
				surface: {
					session_key: "checkpoint",
					processed_report_units: 5000,
					terminal_surface: checkpointFixture.checkpoint,
					is_exited: false,
					exit_code: null,
				},
			},
			ctx,
		);
		const buffer = terminal.buffer.active;
		expect(buffer.type).toBe("alternate");
		expect(buffer.getLine(0)?.translateToString(true).trimEnd()).toBe(
			"ALT-SCREEN",
		);
		expect(buffer.getLine(1)?.translateToString(true).trimEnd()).toBe(
			"WIDE:日本語🙂",
		);
		const wide = buffer.getLine(1)?.getCell(5);
		expect(wide?.getWidth()).toBe(2);
		expect(wide?.isUnderline()).toBeTruthy();
		const red = buffer.getLine(3)?.getCell(0);
		expect(red?.isBold()).toBeTruthy();
		expect(red?.getFgColor()).toBe(1);
		expect(buffer.cursorX).toBe(8);
		expect(buffer.cursorY).toBe(4);
	} finally {
		terminal.dispose();
	}
});
