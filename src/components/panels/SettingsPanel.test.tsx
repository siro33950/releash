import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@/types/settings";
import { SettingsPanel } from "./SettingsPanel";

describe("SettingsPanel", () => {
	const defaultSettings: AppSettings = {
		theme: "dark",
		fontSize: 14,
	};

	it("should render Settings header", () => {
		render(
			<SettingsPanel
				settings={defaultSettings}
				onThemeChange={vi.fn()}
				onFontSizeChange={vi.fn()}
			/>,
		);
		expect(screen.getByText("Settings")).toBeInTheDocument();
	});

	it("should display current theme value", () => {
		render(
			<SettingsPanel
				settings={defaultSettings}
				onThemeChange={vi.fn()}
				onFontSizeChange={vi.fn()}
			/>,
		);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("dark");
	});

	it("should display current font size", () => {
		render(
			<SettingsPanel
				settings={{ ...defaultSettings, fontSize: 18 }}
				onThemeChange={vi.fn()}
				onFontSizeChange={vi.fn()}
			/>,
		);
		expect(screen.getByText("Font Size: 18px")).toBeInTheDocument();
	});

	it("should call onThemeChange when theme is changed", () => {
		const onThemeChange = vi.fn();
		render(
			<SettingsPanel
				settings={defaultSettings}
				onThemeChange={onThemeChange}
				onFontSizeChange={vi.fn()}
			/>,
		);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		expect(onThemeChange).toHaveBeenCalledWith("light");
	});

	it("should call onFontSizeChange when slider is changed", () => {
		const onFontSizeChange = vi.fn();
		render(
			<SettingsPanel
				settings={defaultSettings}
				onThemeChange={vi.fn()}
				onFontSizeChange={onFontSizeChange}
			/>,
		);
		const slider = screen.getByLabelText(/Font Size/);
		fireEvent.change(slider, { target: { value: "20" } });
		expect(onFontSizeChange).toHaveBeenCalledWith(20);
	});

	it("should show light theme option", () => {
		render(
			<SettingsPanel
				settings={{ ...defaultSettings, theme: "light" }}
				onThemeChange={vi.fn()}
				onFontSizeChange={vi.fn()}
			/>,
		);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("light");
	});
});
