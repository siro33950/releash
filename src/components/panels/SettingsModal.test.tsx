import {
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@/types/settings";
import { SettingsModal } from "./SettingsModal";

// Radix UI uses pointer events; jsdom doesn't implement them
beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

describe("SettingsModal", () => {
	beforeEach(async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
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
				case "get_remote_config":
					return Promise.resolve({
						auto_start: false,
						auto_start_on_lan: false,
					});
				case "generate_hooks_config":
					return Promise.resolve('{"hooks":{}}');
				case "get_hooks_status":
					return Promise.resolve("not_configured");
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				case "update_remote_config":
				case "update_notify_config":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});
	});

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
		repoPaths: ["/repos/my-app"],
	};

	it("should render Settings header", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Settings")).toBeInTheDocument();
	});

	it("should display current theme value", () => {
		render(<SettingsModal {...defaultProps} />);
		const trigger = screen.getByRole("combobox", { name: "Theme" });
		expect(trigger).toHaveTextContent("Dark");
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

	it("Save button is enabled after draft change", async () => {
		const user = userEvent.setup();
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", { name: "Auto-update" });
		await user.click(checkbox);
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
	});

	it("should call onSave with updated settings on Save click", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", { name: "Auto-update" });
		await user.click(checkbox);
		const saveBtn = screen.getByRole("button", { name: "Save" });
		await user.click(saveBtn);
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			autoUpdate: false,
		});
	});

	it("should disable Save button after saving AppSettings change", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", { name: "Auto-update" });
		await user.click(checkbox);
		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
		await user.click(saveBtn);
		expect(saveBtn).toBeDisabled();
	});

	it("should show light theme option", () => {
		render(
			<SettingsModal
				{...defaultProps}
				settings={{ ...defaultSettings, theme: "light" }}
			/>,
		);
		const trigger = screen.getByRole("combobox", { name: "Theme" });
		expect(trigger).toHaveTextContent("Light");
	});

	it("should update draft when diff base is changed via select", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Editor"));
		const trigger = screen.getByRole("combobox", { name: "Default Base" });
		await user.click(trigger);
		const option = screen.getByRole("option", { name: "Branch Base" });
		await user.click(option);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith({
			...defaultSettings,
			defaultDiffBase: "branch-base",
		});
	});

	it("should update draft when diff mode is changed via select", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Editor"));
		const trigger = screen.getByRole("combobox", { name: "Default View" });
		await user.click(trigger);
		const option = screen.getByRole("option", { name: "Split" });
		await user.click(option);
		await user.click(screen.getByRole("button", { name: "Save" }));
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

	it("should toggle crash reporting and call onSave", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", {
			name: "Send crash reports",
		});
		await user.click(checkbox);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith(
			expect.objectContaining({ enableCrashReporting: false }),
		);
	});

	it("should display Notifications in nav", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Notifications")).toBeInTheDocument();
	});

	it("should display Webhook URL input field with url type", async () => {
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
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				case "update_remote_config":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Remote"));

		const checkbox = await screen.findByRole("checkbox", {
			name: "Auto-start remote server",
		});
		expect(checkbox).toHaveAttribute("aria-checked", "false");

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
		expect(screen.getByText("Theme")).toBeInTheDocument();
		expect(screen.getByText("Font Size: 14px")).toBeInTheDocument();
	});

	it("should switch sections when nav is clicked", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Theme")).toBeInTheDocument();

		fireEvent.click(screen.getByText("Editor"));
		expect(screen.getByText("Default Base")).toBeInTheDocument();
		expect(screen.queryByText(/^Theme$/)).not.toBeInTheDocument();
	});

	it("should highlight active section in nav", () => {
		render(<SettingsModal {...defaultProps} />);
		const nav = screen.getByRole("navigation");
		const getClasses = (el: Element | null) => el?.className.split(" ") ?? [];

		const appearanceBtn = within(nav).getByText("Appearance").closest("button");
		expect(getClasses(appearanceBtn)).toContain("bg-muted");

		fireEvent.click(within(nav).getByText("Agent"));
		const agentBtn = within(nav).getByText("Agent").closest("button");
		expect(getClasses(agentBtn)).toContain("bg-muted");
		const appearanceBtnAfter = within(nav)
			.getByText("Appearance")
			.closest("button");
		expect(getClasses(appearanceBtnAfter)).not.toContain("bg-muted");
	});

	it("should display Repositories section in nav and switch to it", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_branches":
					return Promise.resolve([
						{ name: "main", is_remote: false },
						{ name: "develop", is_remote: false },
					]);
				case "get_releash_base":
					return Promise.resolve(null);
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
				case "get_remote_config":
					return Promise.resolve({
						auto_start: false,
						auto_start_on_lan: false,
					});
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Repositories")).toBeInTheDocument();
		fireEvent.click(screen.getByText("Repositories"));
		expect(await screen.findByText("Base branch")).toBeInTheDocument();
	});

	it("should resolve hooks loading spinner when agent is claude", async () => {
		const claudeSettings = { ...defaultSettings, agent: "claude" as const };
		render(<SettingsModal {...defaultProps} settings={claudeSettings} />);
		const nav = screen.getByRole("navigation");
		fireEvent.click(within(nav).getByText("Agent"));
		await waitFor(() => {
			expect(screen.getByText("Not configured")).toBeInTheDocument();
		});
	});

	it("should not show permanent spinner when dialog is re-opened with claude agent", async () => {
		const claudeSettings = { ...defaultSettings, agent: "claude" as const };
		const onOpenChange = vi.fn();

		const { rerender } = render(
			<SettingsModal
				{...defaultProps}
				settings={claudeSettings}
				onOpenChange={onOpenChange}
			/>,
		);
		const nav = screen.getByRole("navigation");
		fireEvent.click(within(nav).getByText("Agent"));
		await waitFor(() => {
			expect(screen.getByText("Not configured")).toBeInTheDocument();
		});

		// Close dialog
		rerender(
			<SettingsModal
				{...defaultProps}
				open={false}
				settings={claudeSettings}
				onOpenChange={onOpenChange}
			/>,
		);

		// Re-open dialog
		rerender(
			<SettingsModal
				{...defaultProps}
				open={true}
				settings={claudeSettings}
				onOpenChange={onOpenChange}
			/>,
		);
		fireEvent.click(within(nav).getByText("Agent"));
		await waitFor(() => {
			expect(screen.getByText("Not configured")).toBeInTheDocument();
		});
	});

	it("should save base branch via Apply button", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_branches":
					return Promise.resolve([
						{ name: "main", is_remote: false },
						{ name: "develop", is_remote: false },
					]);
				case "get_releash_base":
					return Promise.resolve(null);
				case "set_releash_base":
					return Promise.resolve(null);
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
				case "get_remote_config":
					return Promise.resolve({
						auto_start: false,
						auto_start_on_lan: false,
					});
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Repositories"));

		const trigger = await screen.findByRole("combobox", {
			name: "Base branch",
		});
		await user.click(trigger);
		const option = screen.getByRole("option", { name: "develop" });
		await user.click(option);

		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
		await user.click(saveBtn);

		expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_releash_base", {
			repoPath: "/repos/my-app",
			base: "develop",
		});
	});
});
