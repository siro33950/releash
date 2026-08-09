import { execFile } from "node:child_process";
import { arch, cpus, platform, totalmem } from "node:os";
import { promisify } from "node:util";
import { appBinaryPath } from "../../wdio.performance.conf";

export interface EchoSamplerPendingMarker {
	marker: string;
	armedAtUnixMs: number;
	armedBaseY: number | null;
	framesWhileArmed: number;
}

export interface EchoSamplerResult {
	marker: string;
	seenAtUnixMs: number;
	armedAtUnixMs: number;
	armedBaseY: number | null;
	seenBaseY: number | null;
	framesWhileArmed: number;
}

// pendingは配列で保持する: 打鍵間隔をechoレイテンシより短くした場合に
// 複数markerが同時にin-flightになるため、単一スロットではarm上書きで
// 計測が喪失する。markerごとにarm時刻とframe数を持ち、可視化された
// markerだけをresultsへ移す。
export interface EchoSampler {
	pending: EchoSamplerPendingMarker[];
	results: EchoSamplerResult[];
	stopped: boolean;
}

export interface WorkspaceSelectionProbe {
	startedAtUnixMs: number;
	bodyFirstUnixMs: number | null;
	contentFirstUnixMs: number | null;
}

declare global {
	interface Window {
		__RELEASH_ECHO_SAMPLER__?: EchoSampler;
		__RELEASH_SELECTION_PROBE__?: WorkspaceSelectionProbe;
	}
}

// WebGLレンダラはDOM rowsを生成しないため、画面内容の検証はxterm bufferを
// probe経由で読む（レンダラ非依存）。
export function terminalBufferContains(needle: string): Promise<boolean> {
	return browser.execute((text) => {
		const readers = window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {};
		return Object.values(readers).some((read) => {
			try {
				return read().text.includes(text);
			} catch {
				return false;
			}
		});
	}, needle);
}

// 累積echo型fixtureでは、redraw・scrollbackにより同一論理行がbuffer内に
// 複数回残り得る。PTYへの二重送信は「1行内に2回以上現れる」形でのみ観測される。
export function terminalBufferLinesContaining(
	needle: string,
): Promise<string[]> {
	return browser.execute((text) => {
		const readers = window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {};
		return Object.values(readers).flatMap((read) => {
			try {
				return read()
					.text.split("\n")
					.filter((line) => line.includes(text));
			} catch {
				return [];
			}
		});
	}, needle);
}

export interface ExecutionConditions {
	buildKind: "release" | "debug" | "unknown";
	machine: {
		platform: string;
		arch: string;
		cpuModel: string;
		totalMemoryBytes: number;
	};
	power: {
		source: "AC" | "Battery" | "unknown";
		batteryPercent: number | null;
	};
}

const execFileAsync = promisify(execFile);

function detectBuildKind(binaryPath: string): ExecutionConditions["buildKind"] {
	if (binaryPath.includes("/release/")) return "release";
	if (binaryPath.includes("/debug/")) return "debug";
	return "unknown";
}

async function detectPowerState(): Promise<ExecutionConditions["power"]> {
	if (platform() !== "darwin") {
		return { source: "unknown", batteryPercent: null };
	}
	try {
		const { stdout } = await execFileAsync("pmset", ["-g", "batt"]);
		const sourceMatch = stdout.match(/'(AC|Battery) Power'/);
		const percentMatch = stdout.match(/(\d+)%/);
		const source =
			sourceMatch?.[1] === "AC"
				? "AC"
				: sourceMatch?.[1] === "Battery"
					? "Battery"
					: "unknown";
		return {
			source,
			batteryPercent: percentMatch ? Number(percentMatch[1]) : null,
		};
	} catch {
		return { source: "unknown", batteryPercent: null };
	}
}

export async function collectExecutionConditions(): Promise<ExecutionConditions> {
	return {
		buildKind: detectBuildKind(appBinaryPath),
		machine: {
			platform: platform(),
			arch: arch(),
			cpuModel: cpus()[0]?.model ?? "unknown",
			totalMemoryBytes: totalmem(),
		},
		power: await detectPowerState(),
	};
}
