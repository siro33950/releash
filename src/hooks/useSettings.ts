import { useCallback, useEffect, useState } from "react";
import {
	DEFAULT_SETTINGS,
	type AppSettings,
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

	useEffect(() => {
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

	return {
		settings,
		updateTheme,
		updateFontSize,
		updateDefaultDiffBase,
		updateDefaultDiffMode,
	};
}
