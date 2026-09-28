import { firstState } from "@/lib/client";

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

let cached: Promise<TerminalPerformanceSwitches> | null = null;

export function getTerminalPerformanceSwitches(): Promise<TerminalPerformanceSwitches> {
	cached ??= firstState("performance-switches")
		.then((switches) => switches.terminal)
		.catch((error) => {
			console.warn(
				"Failed to load terminal performance switches, using defaults:",
				error,
			);
			cached = null;
			return DEFAULT_TERMINAL_PERFORMANCE_SWITCHES;
		});
	return cached;
}

export function resetTerminalPerformanceSwitchesCache(): void {
	cached = null;
}
