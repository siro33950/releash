import { createCachedInvoke } from "./cachedInvoke";

export interface TerminalPerformanceSwitches {
	disableOutputFlowControl: boolean;
	disableTerminalJournal: boolean;
	disableTerminalWebsocket: boolean;
	disableRendererWriteSerialization: boolean;
	disableWebglRenderer: boolean;
}

export const DEFAULT_TERMINAL_PERFORMANCE_SWITCHES: TerminalPerformanceSwitches =
	{
		disableOutputFlowControl: false,
		disableTerminalJournal: false,
		disableTerminalWebsocket: false,
		disableRendererWriteSerialization: false,
		disableWebglRenderer: false,
	};

const cachedSwitches = createCachedInvoke<
	TerminalPerformanceSwitches | null,
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
