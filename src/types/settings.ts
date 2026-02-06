export type Theme = "dark" | "light";
export type DiffBase = "HEAD" | "staged";
export type DiffMode = "gutter" | "inline" | "split";

export interface AppSettings {
	theme: Theme;
	fontSize: number;
	defaultDiffBase: DiffBase;
	defaultDiffMode: DiffMode;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
	defaultDiffBase: "staged",
	defaultDiffMode: "inline",
};
