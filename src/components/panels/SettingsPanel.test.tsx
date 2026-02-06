import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@/types/settings";
import { SettingsPanel } from "./SettingsPanel";

describe("SettingsPanel", () => {
	const defaultSettings: AppSettings = {
		theme: "dark",
		fontSize: 14,
		defaultDiffBase: "staged",
		defaultDiffMode: "inline",
	};

	const defaultProps = {
		settings: defaultSettings,
		onThemeChange: vi.fn(),
		onFontSizeChange: vi.fn(),
		onDiffBaseChange: vi.fn(),
		onDiffModeChange: vi.fn(),
	};

	it("should render Settings header", () => {
		render(<SettingsPanel {...defaultProps} />);
		expect(screen.getByText("Settings")).toBeInTheDocument();
	});

	it("should display current theme value", () => {
		render(<SettingsPanel {...defaultProps} />);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("dark");
	});

	it("should display current font size", () => {
		render(
			<SettingsPanel
				{...defaultProps}
				settings={{ ...defaultSettings, fontSize: 18 }}
			/>,
		);
		expect(screen.getByText("Font Size: 18px")).toBeInTheDocument();
	});

	it("should call onThemeChange when theme is changed", () => {
		const onThemeChange = vi.fn();
		render(<SettingsPanel {...defaultProps} onThemeChange={onThemeChange} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		expect(onThemeChange).toHaveBeenCalledWith("light");
	});

	it("should call onFontSizeChange when slider is changed", () => {
		const onFontSizeChange = vi.fn();
		render(
			<SettingsPanel {...defaultProps} onFontSizeChange={onFontSizeChange} />,
		);
		const slider = screen.getByLabelText(/Font Size/);
		fireEvent.change(slider, { target: { value: "20" } });
		expect(onFontSizeChange).toHaveBeenCalledWith(20);
	});

	it("should show light theme option", () => {
		render(
			<SettingsPanel
				{...defaultProps}
				settings={{ ...defaultSettings, theme: "light" }}
			/>,
		);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("light");
	});

	it("should call onDiffBaseChange when base is changed", () => {
		const onDiffBaseChange = vi.fn();
		render(
			<SettingsPanel {...defaultProps} onDiffBaseChange={onDiffBaseChange} />,
		);
		const select = screen.getByLabelText("Default Base");
		fireEvent.change(select, { target: { value: "HEAD" } });
		expect(onDiffBaseChange).toHaveBeenCalledWith("HEAD");
	});

	it("should call onDiffModeChange when view is changed", () => {
		const onDiffModeChange = vi.fn();
		render(
			<SettingsPanel {...defaultProps} onDiffModeChange={onDiffModeChange} />,
		);
		const select = screen.getByLabelText("Default View");
		fireEvent.change(select, { target: { value: "split" } });
		expect(onDiffModeChange).toHaveBeenCalledWith("split");
	});
});
