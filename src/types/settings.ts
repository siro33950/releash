export type Theme = "dark" | "light";
export type DiffBase = "branch-base" | "head";
export type DiffSection = "changes" | "staged";
export type DiffMode = "gutter" | "inline" | "split";

export interface AppSettings {
	theme: Theme;
	fontSize: number;
	defaultDiffBase: DiffBase;
	defaultDiffMode: DiffMode;
	terminalStartupCommand: string;
	autoUpdate: boolean;
	performanceTelemetry: boolean;
	enableCrashReporting: boolean;
	defaultDiffOnlyMode: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
	defaultDiffBase: "head",
	defaultDiffMode: "inline",
	defaultDiffOnlyMode: false,
	terminalStartupCommand: "",
	autoUpdate: true,
	performanceTelemetry: true,
	enableCrashReporting: true,
};
