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
		agent: "none",
		agentAutoApprove: false,
		terminalStartupCommand: "",
		autoUpdate: true,
	};

	const defaultProps = {
		settings: defaultSettings,
		onSave: vi.fn(),
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

	it("Save button is disabled when no changes", () => {
		render(<SettingsPanel {...defaultProps} />);
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeDisabled();
	});

	it("Save button is enabled after draft change", () => {
		render(<SettingsPanel {...defaultProps} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
	});

	it("should call onSave with updated settings on Save click", () => {
		const onSave = vi.fn();
		render(<SettingsPanel {...defaultProps} onSave={onSave} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		const saveBtn = screen.getByRole("button", { name: "Save" });
		fireEvent.click(saveBtn);
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			theme: "light",
		});
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

	it("should update draft when diff base is changed", () => {
		const onSave = vi.fn();
		render(<SettingsPanel {...defaultProps} onSave={onSave} />);
		const select = screen.getByLabelText("Default Base");
		fireEvent.change(select, { target: { value: "HEAD" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			defaultDiffBase: "HEAD",
		});
	});

	it("should update draft when diff mode is changed", () => {
		const onSave = vi.fn();
		render(<SettingsPanel {...defaultProps} onSave={onSave} />);
		const select = screen.getByLabelText("Default View");
		fireEvent.change(select, { target: { value: "split" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			defaultDiffMode: "split",
		});
	});
});
