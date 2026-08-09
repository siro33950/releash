import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
	getTerminalPerformanceSwitches,
	resetTerminalPerformanceSwitchesCache,
} from "./terminalPerformanceSwitches";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

describe("getTerminalPerformanceSwitches", () => {
	beforeEach(() => {
		invokeMock.mockReset();
		resetTerminalPerformanceSwitchesCache();
	});

	it("backendのswitch値を返しresultをcacheする", async () => {
		invokeMock.mockResolvedValue({
			disableOutputFlowControl: true,
			disableTerminalJournal: false,
			disableTerminalWebsocket: false,
			disableRendererWriteSerialization: true,
		});

		const first = await getTerminalPerformanceSwitches();
		const second = await getTerminalPerformanceSwitches();

		expect(first.disableOutputFlowControl).toBe(true);
		expect(first.disableRendererWriteSerialization).toBe(true);
		expect(second).toBe(first);
		expect(invokeMock).toHaveBeenCalledTimes(1);
		expect(invokeMock).toHaveBeenCalledWith(
			"get_terminal_performance_switches",
		);
	});

	it("invoke失敗時はdefaultへfallbackしcacheを破棄して次回再試行する", async () => {
		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
		invokeMock.mockRejectedValueOnce(new Error("unavailable"));

		const switches = await getTerminalPerformanceSwitches();

		expect(switches).toEqual(DEFAULT_TERMINAL_PERFORMANCE_SWITCHES);
		expect(warnSpy).toHaveBeenCalledTimes(1);

		invokeMock.mockResolvedValueOnce({
			...DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
			disableWebglRenderer: true,
		});
		const retried = await getTerminalPerformanceSwitches();

		expect(retried.disableWebglRenderer).toBe(true);
		expect(invokeMock).toHaveBeenCalledTimes(2);
		warnSpy.mockRestore();
	});
});
