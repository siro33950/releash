import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@/types/settings";
import { SettingsModal } from "./SettingsModal";

describe("SettingsModal", () => {
	const defaultSettings: AppSettings = {
		theme: "dark",
		fontSize: 14,
		defaultDiffBase: "staged",
		defaultDiffMode: "inline",
		agent: "none",
		agentAutoApprove: false,
		terminalStartupCommand: "",
		autoUpdate: true,
		telemetryEnabled: true,
		enableCrashReporting: true,
	};

	const defaultProps = {
		open: true,
		onOpenChange: vi.fn(),
		settings: defaultSettings,
		onSave: vi.fn(),
	};

	it("should render Settings header", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Settings")).toBeInTheDocument();
	});

	it("should display current theme value", () => {
		render(<SettingsModal {...defaultProps} />);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("dark");
	});

	it("should display current font size", () => {
		render(
			<SettingsModal
				{...defaultProps}
				settings={{ ...defaultSettings, fontSize: 18 }}
			/>,
		);
		expect(screen.getByText("Font Size: 18px")).toBeInTheDocument();
	});

	it("Save button is disabled when no changes", () => {
		render(<SettingsModal {...defaultProps} />);
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeDisabled();
	});

	it("Save button is enabled after draft change", () => {
		render(<SettingsModal {...defaultProps} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
	});

	it("should call onSave with updated settings on Save click", () => {
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		const saveBtn = screen.getByRole("button", { name: "Save" });
		fireEvent.click(saveBtn);
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			theme: "light",
		});
	});

	it("should disable Save button after saving AppSettings change", () => {
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		const select = screen.getByLabelText("Theme");
		fireEvent.change(select, { target: { value: "light" } });
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
		fireEvent.click(saveBtn);
		expect(saveBtn).toBeDisabled();
	});

	it("should show light theme option", () => {
		render(
			<SettingsModal
				{...defaultProps}
				settings={{ ...defaultSettings, theme: "light" }}
			/>,
		);
		const select = screen.getByLabelText("Theme") as HTMLSelectElement;
		expect(select.value).toBe("light");
	});

	it("should navigate to Editor section and update diff base", () => {
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Editor"));
		const select = screen.getByLabelText("Default Base");
		fireEvent.change(select, { target: { value: "HEAD" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			defaultDiffBase: "HEAD",
		});
	});

	it("should navigate to Editor section and update diff mode", () => {
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Editor"));
		const select = screen.getByLabelText("Default View");
		fireEvent.change(select, { target: { value: "split" } });
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			defaultDiffMode: "split",
		});
	});

	it("should navigate to Privacy section and display crash reporting toggle", () => {
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		expect(screen.getByText("Send crash reports")).toBeInTheDocument();
	});

	it("should toggle crash reporting and call onSave", () => {
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByLabelText("Send crash reports");
		fireEvent.click(checkbox);
		fireEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith(
			expect.objectContaining({ enableCrashReporting: false }),
		);
	});

	it("should display Notifications in nav", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Notifications")).toBeInTheDocument();
	});

	it("should display Webhook URL input field with url type", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue({
			webhook_url: "",
			on_running: false,
			on_done: true,
			on_error: true,
			on_waiting: true,
			desktop_mode: "always",
			inactive_timeout_minutes: 2,
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Notifications"));
		const input = await screen.findByLabelText("Webhook URL");
		expect(input).toBeInTheDocument();
		expect(input).toHaveAttribute("type", "url");
	});

	it("should display Remote in nav", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Remote")).toBeInTheDocument();
	});

	it("should enable Save when remote auto-start is toggled, and disable after Save", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const onSave = vi.fn();

		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_remote_config":
					return Promise.resolve({
						auto_start: false,
						auto_start_on_lan: false,
					});
				case "get_notify_config":
					return Promise.resolve({
						webhook_url: "",
						on_running: false,
						on_done: true,
						on_error: true,
						on_waiting: true,
						desktop_mode: "always",
						inactive_timeout_minutes: 2,
					});
				case "update_remote_config":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Remote"));

		const checkbox = await screen.findByLabelText("Auto-start remote server");
		expect(checkbox).not.toBeChecked();

		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeDisabled();

		fireEvent.click(checkbox);
		expect(saveBtn).toBeEnabled();

		fireEvent.click(saveBtn);
		await vi.waitFor(() => {
			expect(saveBtn).toBeDisabled();
		});
	});

	it("should show Appearance section by default", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByLabelText("Theme")).toBeInTheDocument();
		expect(screen.getByText("Font Size: 14px")).toBeInTheDocument();
	});

	it("should switch sections when nav is clicked", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByLabelText("Theme")).toBeInTheDocument();

		fireEvent.click(screen.getByText("Editor"));
		expect(screen.getByLabelText("Default Base")).toBeInTheDocument();
		expect(screen.queryByLabelText("Theme")).not.toBeInTheDocument();
	});

	it("should highlight active section in nav", () => {
		render(<SettingsModal {...defaultProps} />);
		const nav = screen.getByRole("navigation");
		const getClasses = (el: Element | null) => el?.className.split(" ") ?? [];

		const appearanceBtn = within(nav).getByText("Appearance").closest("button");
		expect(getClasses(appearanceBtn)).toContain("bg-accent");

		fireEvent.click(within(nav).getByText("Agent"));
		const agentBtn = within(nav).getByText("Agent").closest("button");
		expect(getClasses(agentBtn)).toContain("bg-accent");
		const appearanceBtnAfter = within(nav)
			.getByText("Appearance")
			.closest("button");
		expect(getClasses(appearanceBtnAfter)).not.toContain("bg-accent");
	});
});
