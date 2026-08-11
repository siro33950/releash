export type TerminalSurfaceOwner =
	| { kind: "workspace"; workspacePath: string }
	| { kind: "session"; workspacePath: string; sessionId: string };

export interface TerminalSurfaceSnapshot {
	replay: string;
	sequence: number;
	cols: number;
	rows: number;
}

export interface TerminalSnapshotSurface {
	session_key: string;
	terminal_surface: TerminalSurfaceSnapshot;
	is_exited: boolean;
	exit_code: number | null;
}

export type TerminalSurfaceStreamItem =
	| {
			type: "snapshot";
			surface: TerminalSnapshotSurface;
	  }
	| {
			type: "output";
			session_key: string;
			data: string;
			sequence: number;
	  }
	| {
			type: "resize";
			session_key: string;
			cols: number;
			rows: number;
			sequence: number;
	  }
	| {
			type: "exit";
			session_key: string;
			exit_code: number | null;
			sequence: number;
	  }
	| {
			type: "input_unavailable";
			session_key: string;
			message: string;
	  };

export type TerminalOutputTracePhase =
	| "channel_receive"
	| "xterm_parsed"
	| "paint";

export interface TerminalStreamApplyContext {
	isCurrent(): boolean;
	drainLiveOutput(): Promise<void>;
	resizeTerminal(cols: number, rows: number): void;
	writeToTerminal(data: string): Promise<void>;
	applySnapshotIdentity(sessionKey: string): void;
	syncPtySizeAfterEmptySnapshot(): void;
	reportSnapshotReplayParsed(sequence: number): void;
	completeRecovery(): void;
	setRunning(running: boolean): void;
	completeInitialSnapshot(): void;
	flushStartupInput(): void;
	takeOutputTraceSequence(): number | undefined;
	reportOutputTracePoint(
		sequence: number,
		phase: TerminalOutputTracePhase,
	): void;
	enqueueOutput(data: string, onParsed: () => void): void;
	acknowledgeOutput(sequence: number): void;
	reportInputUnavailable(message: string): void;
}

function processExitNotice(exitCode: number | null): string {
	return `\r\n\x1b[90m[Process exited with code ${exitCode ?? "unknown"}]\x1b[0m\r\n`;
}

export async function applyTerminalStreamItem(
	item: TerminalSurfaceStreamItem,
	ctx: TerminalStreamApplyContext,
): Promise<void> {
	if (!ctx.isCurrent()) return;
	if (item.type === "snapshot") {
		await ctx.drainLiveOutput();
		if (!ctx.isCurrent()) return;
		ctx.applySnapshotIdentity(item.surface.session_key);
		const checkpoint = item.surface.terminal_surface;
		if (checkpoint.replay) {
			// replayは記録時の寸法で描画する必要がある
			ctx.resizeTerminal(checkpoint.cols, checkpoint.rows);
			await ctx.writeToTerminal(checkpoint.replay);
			ctx.reportSnapshotReplayParsed(checkpoint.sequence);
		} else {
			// 新規サーフェスに保存画面は無い。fit済みの実サイズを維持し、
			// providerの初回描画前にPTY寸法を確定させて二重描画を防ぐ
			ctx.syncPtySizeAfterEmptySnapshot();
		}
		ctx.completeRecovery();
		ctx.setRunning(!item.surface.is_exited);
		if (item.surface.is_exited) {
			await ctx.writeToTerminal(processExitNotice(item.surface.exit_code));
		}
		ctx.completeInitialSnapshot();
		ctx.flushStartupInput();
		return;
	}
	if (item.type === "output") {
		const traceSequence = ctx.takeOutputTraceSequence();
		if (traceSequence !== undefined) {
			ctx.reportOutputTracePoint(traceSequence, "channel_receive");
		}
		ctx.enqueueOutput(item.data, () => {
			ctx.acknowledgeOutput(item.sequence);
			if (traceSequence !== undefined) {
				ctx.reportOutputTracePoint(traceSequence, "xterm_parsed");
				requestAnimationFrame(() => {
					ctx.reportOutputTracePoint(traceSequence, "paint");
				});
			}
		});
		return;
	}
	if (item.type === "resize") {
		await ctx.drainLiveOutput();
		if (!ctx.isCurrent()) return;
		ctx.resizeTerminal(item.cols, item.rows);
		return;
	}
	if (item.type === "input_unavailable") {
		ctx.reportInputUnavailable(item.message);
		return;
	}
	if (item.type === "exit") {
		await ctx.drainLiveOutput();
		if (!ctx.isCurrent()) return;
		await ctx.writeToTerminal(processExitNotice(item.exit_code));
		ctx.setRunning(false);
	}
}
