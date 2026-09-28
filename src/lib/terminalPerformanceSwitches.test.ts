import { beforeEach, describe, expect, it, vi } from "vitest";
import { firstState } from "@/lib/client";
import {
	DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
	getTerminalPerformanceSwitches,
	resetTerminalPerformanceSwitchesCache,
} from "./terminalPerformanceSwitches";

vi.mock("@/lib/client", () => ({
	firstState: vi.fn(),
}));

const firstStateMock = vi.mocked(firstState);

describe("getTerminalPerformanceSwitches", () => {
	beforeEach(() => {
		firstStateMock.mockReset();
		resetTerminalPerformanceSwitchesCache();
	});

	it("購読の最初の値からterminalのswitch値を返しresultをcacheする", async () => {
		firstStateMock.mockResolvedValue({
			realAppMode: false,
			terminal: {
				disableOutputFlowControl: true,
				disableTerminalJournal: false,
				disableRendererWriteSerialization: true,
				disableWebglRenderer: false,
			},
		});

		const first = await getTerminalPerformanceSwitches();
		const second = await getTerminalPerformanceSwitches();

		expect(first.disableOutputFlowControl).toBe(true);
		expect(first.disableRendererWriteSerialization).toBe(true);
		expect(second).toBe(first);
		expect(firstStateMock).toHaveBeenCalledTimes(1);
		expect(firstStateMock).toHaveBeenCalledWith("performance-switches");
	});

	it("取得失敗時はdefaultへfallbackしcacheを破棄して次回再試行する", async () => {
		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
		firstStateMock.mockRejectedValueOnce(new Error("unavailable"));

		const switches = await getTerminalPerformanceSwitches();

		expect(switches).toEqual(DEFAULT_TERMINAL_PERFORMANCE_SWITCHES);
		expect(warnSpy).toHaveBeenCalledTimes(1);

		firstStateMock.mockResolvedValueOnce({
			realAppMode: false,
			terminal: {
				...DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
				disableWebglRenderer: true,
			},
		});
		const retried = await getTerminalPerformanceSwitches();

		expect(retried.disableWebglRenderer).toBe(true);
		expect(firstStateMock).toHaveBeenCalledTimes(2);
		warnSpy.mockRestore();
	});
});
