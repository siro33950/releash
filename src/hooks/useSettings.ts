import { invoke } from "@tauri-apps/api/core";
import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { setSentryEnabled } from "@/lib/sentry";
import {
	type AgentType,
	type AppSettings,
	DEFAULT_SETTINGS,
	type DiffBase,
	type DiffMode,
	type Theme,
} from "@/types/settings";

const STORAGE_KEY = "releash-settings";

function loadSettings(): AppSettings {
	try {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored) {
			const parsed = JSON.parse(stored) as Partial<AppSettings>;
			// Migration: "staged" was removed from DiffBase
			if ((parsed as Record<string, unknown>).defaultDiffBase === "staged") {
				parsed.defaultDiffBase = "head";
			}
			return { ...DEFAULT_SETTINGS, ...parsed };
		}
	} catch {
		// ignore
	}
	return { ...DEFAULT_SETTINGS };
}

function saveSettings(settings: AppSettings): void {
	localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

function applyTheme(theme: Theme): void {
	const root = document.documentElement;
	if (theme === "light") {
		root.classList.add("light");
		root.classList.remove("dark");
	} else {
		root.classList.add("dark");
		root.classList.remove("light");
	}
}

export function useSettings() {
	const [settings, setSettings] = useState<AppSettings>(loadSettings);

	useLayoutEffect(() => {
		applyTheme(settings.theme);
	}, [settings.theme]);

	useEffect(() => {
		saveSettings(settings);
	}, [settings]);

	const updateTheme = useCallback((theme: Theme) => {
		setSettings((prev) => ({ ...prev, theme }));
	}, []);

	const updateFontSize = useCallback((fontSize: number) => {
		setSettings((prev) => ({ ...prev, fontSize }));
	}, []);

	const updateDefaultDiffBase = useCallback((defaultDiffBase: DiffBase) => {
		setSettings((prev) => ({ ...prev, defaultDiffBase }));
	}, []);

	const updateDefaultDiffMode = useCallback((defaultDiffMode: DiffMode) => {
		setSettings((prev) => ({ ...prev, defaultDiffMode }));
	}, []);

	const updateTerminalStartupCommand = useCallback(
		(terminalStartupCommand: string) => {
			setSettings((prev) => ({ ...prev, terminalStartupCommand }));
		},
		[],
	);

	const updateAgent = useCallback((agent: AgentType) => {
		setSettings((prev) => ({ ...prev, agent }));
	}, []);

	const updateAgentAutoApprove = useCallback((agentAutoApprove: boolean) => {
		setSettings((prev) => ({ ...prev, agentAutoApprove }));
	}, []);

	const updateTelemetryEnabled = useCallback((telemetryEnabled: boolean) => {
		setSettings((prev) => ({ ...prev, telemetryEnabled }));
	}, []);

	const prevCrashReporting = useRef(settings.enableCrashReporting);

	const updateSettings = useCallback((next: AppSettings) => {
		if (next.enableCrashReporting !== prevCrashReporting.current) {
			prevCrashReporting.current = next.enableCrashReporting;
			setSentryEnabled(next.enableCrashReporting);
			invoke("update_crash_reporting", {
				enabled: next.enableCrashReporting,
			}).catch(() => {});
		}
		setSettings(next);
	}, []);

	return {
		settings,
		updateTheme,
		updateFontSize,
		updateDefaultDiffBase,
		updateDefaultDiffMode,
		updateTerminalStartupCommand,
		updateAgent,
		updateAgentAutoApprove,
		updateTelemetryEnabled,
		updateSettings,
	};
}
