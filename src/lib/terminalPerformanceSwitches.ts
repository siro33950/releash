import { createCachedInvoke } from "./cachedInvoke";

export interface TerminalPerformanceSwitches {
	disableOutputFlowControl: boolean;
	disableTerminalJournal: boolean;
	disableRendererWriteSerialization: boolean;
	disableWebglRenderer: boolean;
}

export const DEFAULT_TERMINAL_PERFORMANCE_SWITCHES: TerminalPerformanceSwitches =
	{
		disableOutputFlowControl: false,
		disableTerminalJournal: false,
		disableRendererWriteSerialization: false,
		disableWebglRenderer: false,
	};

const cachedSwitches = createCachedInvoke<
	"get_terminal_performance_switches",
	TerminalPerformanceSwitches
>({
	command: "get_terminal_performance_switches",
	normalize: (switches) => switches ?? DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
	fallback: DEFAULT_TERMINAL_PERFORMANCE_SWITCHES,
	failureMessage:
		"Failed to load terminal performance switches, using defaults:",
});

export function getTerminalPerformanceSwitches(): Promise<TerminalPerformanceSwitches> {
	return cachedSwitches.get();
}

export function resetTerminalPerformanceSwitchesCache(): void {
	cachedSwitches.reset();
}
