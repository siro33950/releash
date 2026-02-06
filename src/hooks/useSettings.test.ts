import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSettings } from "./useSettings";

describe("useSettings", () => {
	beforeEach(() => {
		localStorage.clear();
		document.documentElement.classList.remove("light", "dark");
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("should return default settings when localStorage is empty", () => {
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings).toEqual({
			theme: "dark",
			fontSize: 14,
			defaultDiffBase: "staged",
			defaultDiffMode: "inline",
		});
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
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ fontSize: 20 }),
		);
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings.theme).toBe("dark");
		expect(result.current.settings.fontSize).toBe(20);
	});

	it("should handle invalid JSON in localStorage", () => {
		localStorage.setItem("releash-settings", "not-json");
		const { result } = renderHook(() => useSettings());
		expect(result.current.settings).toEqual({
			theme: "dark",
			fontSize: 14,
			defaultDiffBase: "staged",
			defaultDiffMode: "inline",
		});
	});

	it("should save settings to localStorage on change", () => {
		const { result } = renderHook(() => useSettings());

		act(() => {
			result.current.updateFontSize(20);
		});

		const stored = JSON.parse(
			localStorage.getItem("releash-settings") ?? "{}",
		);
		expect(stored.fontSize).toBe(20);
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
});
