import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "@/types/settings";
import { useSettings } from "./useSettings";

describe("useSettings", () => {
	beforeEach(() => {
		localStorage.clear();
		document.documentElement.classList.remove("light", "dark");
		vi.clearAllMocks();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("should return default settings when localStorage is empty", () => {
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings).toEqual(DEFAULT_SETTINGS);
	});

	it("should load settings from localStorage", () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ theme: "light", fontSize: 18 }),
		);
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.theme).toBe("light");
		expect(result.current.settings.fontSize).toBe(18);
	});

	it("should merge partial settings with defaults", () => {
		localStorage.setItem("releash-settings", JSON.stringify({ fontSize: 20 }));
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.theme).toBe("dark");
		expect(result.current.settings.fontSize).toBe(20);
	});

	it("should handle invalid JSON in localStorage", () => {
		localStorage.setItem("releash-settings", "not-json");
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings).toEqual(DEFAULT_SETTINGS);
	});

	it("should save settings to localStorage on change", () => {
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateFontSize(20);
		});

		const stored = JSON.parse(localStorage.getItem("releash-settings") ?? "{}");
		expect(stored.fontSize).toBe(20);
		expect(stored).not.toHaveProperty("performanceTelemetry");
	});

	it("should update theme and apply class to document", () => {
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateTheme("light");
		});

		expect(result.current.settings.theme).toBe("light");
		expect(document.documentElement.classList.contains("light")).toBe(true);
		expect(document.documentElement.classList.contains("dark")).toBe(false);
	});

	it("should switch back to dark theme", () => {
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateTheme("light");
		});
		act(() => {
			result.current.updateTheme("dark");
		});

		expect(result.current.settings.theme).toBe("dark");
		expect(document.documentElement.classList.contains("dark")).toBe(true);
		expect(document.documentElement.classList.contains("light")).toBe(false);
	});

	it("should update fontSize", () => {
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateFontSize(18);
		});

		expect(result.current.settings.fontSize).toBe(18);
	});

	it("should default enableCrashReporting to true", () => {
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.enableCrashReporting).toBe(true);
	});

	it("should load enableCrashReporting from localStorage", () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ enableCrashReporting: false }),
		);
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.enableCrashReporting).toBe(false);
	});

	it("should not load performanceTelemetry from localStorage", () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: false, telemetryEnabled: false }),
		);
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.performanceTelemetry).toBe(
			DEFAULT_SETTINGS.performanceTelemetry,
		);
	});

	it("should remove performance telemetry keys from persisted settings", () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: false, telemetryEnabled: false }),
		);
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateFontSize(20);
		});

		const stored = JSON.parse(localStorage.getItem("releash-settings") ?? "{}");
		expect(stored.fontSize).toBe(20);
		expect(stored).not.toHaveProperty("performanceTelemetry");
		expect(stored).not.toHaveProperty("telemetryEnabled");
	});

	it("should initialize performanceTelemetry from Rust without writing localStorage value back", async () => {
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "get_performance_telemetry_enabled") {
				return Promise.resolve(false);
			}
			return Promise.resolve(null);
		});
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: true }),
		);

		const { result } = renderHook(() => useSettings());

		await waitFor(() => {
			expect(result.current.settings.performanceTelemetry).toBe(false);
		});
		const stored = JSON.parse(localStorage.getItem("releash-settings") ?? "{}");
		expect(stored).not.toHaveProperty("performanceTelemetry");
		expect(invoke).toHaveBeenCalledWith("get_performance_telemetry_enabled");
		expect(invoke).not.toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: true,
		});
	});

	it("should keep Rust opt-out when localStorage is corrupt", async () => {
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "get_performance_telemetry_enabled") {
				return Promise.resolve(false);
			}
			return Promise.resolve(null);
		});
		localStorage.setItem("releash-settings", "not-json");

		const { result } = renderHook(() => useSettings());

		await waitFor(() => {
			expect(result.current.settings.performanceTelemetry).toBe(false);
		});
		expect(invoke).not.toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: true,
		});
	});
});
