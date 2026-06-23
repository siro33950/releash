import { invoke } from "@tauri-apps/api/core";
import {
	useCallback,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import {
	type AgentType,
	type AppSettings,
	DEFAULT_SETTINGS,
	type DiffBase,
	type DiffMode,
	type Theme,
} from "@/types/settings";

const STORAGE_KEY = "releash-settings";

type StoredSettings = Partial<AppSettings> & {
	telemetryEnabled?: unknown;
};

function loadSettings(): AppSettings {
	try {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored) {
			const parsed = JSON.parse(stored) as StoredSettings;
			// Migration: "staged" was removed from DiffBase
			if ((parsed as Record<string, unknown>).defaultDiffBase === "staged") {
				parsed.defaultDiffBase = "head";
			}
			// Rust app_config is the source of truth for performance telemetry.
			delete parsed.performanceTelemetry;
			delete parsed.telemetryEnabled;
			return { ...DEFAULT_SETTINGS, ...parsed };
		}
	} catch {
		// ignore
	}
	return { ...DEFAULT_SETTINGS };
}

function saveSettings(settings: AppSettings): void {
	const { performanceTelemetry: _performanceTelemetry, ...storedSettings } =
		settings;
	localStorage.setItem(STORAGE_KEY, JSON.stringify(storedSettings));
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
		let cancelled = false;
		invoke<boolean>("get_performance_telemetry_enabled")
			.then((enabled) => {
				if (cancelled || typeof enabled !== "boolean") return;
				setSettings((prev) =>
					prev.performanceTelemetry === enabled
						? prev
						: { ...prev, performanceTelemetry: enabled },
				);
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	}, []);

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

	const updateDefaultDiffOnlyMode = useCallback(
		(defaultDiffOnlyMode: boolean) => {
			setSettings((prev) => ({ ...prev, defaultDiffOnlyMode }));
		},
		[],
	);

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

	const updatePerformanceTelemetry = useCallback(
		(performanceTelemetry: boolean) => {
			setSettings((prev) => ({ ...prev, performanceTelemetry }));
		},
		[],
	);

	const prevCrashReporting = useRef(settings.enableCrashReporting);

	const updateSettings = useCallback((next: AppSettings) => {
		if (next.enableCrashReporting !== prevCrashReporting.current) {
			prevCrashReporting.current = next.enableCrashReporting;
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
		updateDefaultDiffOnlyMode,
		updateTerminalStartupCommand,
		updateAgent,
		updateAgentAutoApprove,
		updatePerformanceTelemetry,
		updateSettings,
	};
}
