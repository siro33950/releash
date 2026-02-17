export type DesktopNotifyMode = "always" | "when_inactive";

export interface NotifyConfig {
	webhook_url: string;
	on_running: boolean;
	on_done: boolean;
	on_error: boolean;
	on_waiting: boolean;
	desktop_mode: DesktopNotifyMode;
	inactive_timeout_minutes: number;
}

export const INACTIVE_TIMEOUT_OPTIONS = [
	{ value: 1, label: "1 min" },
	{ value: 2, label: "2 min" },
	{ value: 5, label: "5 min" },
	{ value: 10, label: "10 min" },
] as const;
