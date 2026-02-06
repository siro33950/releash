export type Theme = "dark" | "light";

export interface AppSettings {
	theme: Theme;
	fontSize: number;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
};
