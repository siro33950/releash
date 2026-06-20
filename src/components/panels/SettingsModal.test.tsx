import {
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { type AppSettings, DEFAULT_SETTINGS } from "@/types/settings";
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
				case "get_workflow_config":
					return Promise.resolve({
						approval_auto_approve: false,
					});
				case "generate_hooks_config":
					return Promise.resolve('{"hooks":{}}');
				case "get_hooks_status":
					return Promise.resolve("not_configured");
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "get_configured_agents":
					return Promise.resolve([]);
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				case "get_agent_shortcut_settings":
					return Promise.resolve([
						{
							id: "command_menu",
							label: "Command menu",
							shortcut: "Cmd K",
							alternateShortcut: "Cmd Shift P",
							defaultShortcut: "Cmd K",
						},
					]);
				case "update_workflow_config":
				case "update_notify_config":
				case "update_agent_shortcut_settings":
				case "reset_agent_shortcut_settings":
					return Promise.resolve(null);
				case "get_external_editor":
					return Promise.resolve("");
				case "detect_editors":
					return Promise.resolve([]);
				case "list_workflows":
					return Promise.resolve([]);
				case "diagnose_all_cmd":
					return Promise.resolve({
						items: [],
						workflow_summaries: {},
						facet_summaries: {},
						facet_usage: {},
					});
				default:
					return Promise.resolve(null);
			}
		});
	});

	const defaultSettings: AppSettings = { ...DEFAULT_SETTINGS };

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

	it("saves agent shortcut customization through Rust settings", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));

		const commandMenuInput = await screen.findByLabelText(/Command menu/);
		await user.clear(commandMenuInput);
		await user.type(commandMenuInput, "Ctrl Shift K");
		await user.click(screen.getByRole("button", { name: "Save" }));

		await waitFor(() =>
			expect(invoke).toHaveBeenCalledWith("update_agent_shortcut_settings", {
				shortcuts: expect.arrayContaining([
					expect.objectContaining({
						id: "command_menu",
						shortcut: "Ctrl Shift K",
					}),
				]),
			}),
		);
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
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "get_configured_agents":
					return Promise.resolve([]);
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

	it("should load and save workflow approval auto-approve independently from agent auto-approve", async () => {
		const user = userEvent.setup();
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
				case "get_workflow_config":
					return Promise.resolve({
						approval_auto_approve: true,
					});
				case "generate_hooks_config":
					return Promise.resolve('{"hooks":{}}');
				case "get_hooks_status":
					return Promise.resolve("not_configured");
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "get_configured_agents":
					return Promise.resolve([]);
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				case "update_workflow_config":
				case "update_notify_config":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		render(
			<SettingsModal
				{...defaultProps}
				settings={{
					...defaultSettings,
					agent: "codex",
					agentAutoApprove: false,
				}}
			/>,
		);
		const nav = screen.getByRole("navigation");
		fireEvent.click(within(nav).getByText("Agent"));
		const workflowCheckbox = await screen.findByRole("checkbox", {
			name: "Workflow approval auto-approve",
		});
		const agentCheckbox = screen.getByRole("checkbox", {
			name: "Auto-approve",
		});
		await waitFor(() => {
			expect(workflowCheckbox).toBeChecked();
		});
		expect(agentCheckbox).not.toBeChecked();

		await user.click(workflowCheckbox);
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(invoke).toHaveBeenCalledWith("update_workflow_config", {
			workflow: { approval_auto_approve: false },
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

	it("should save external editor selection via Save button", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_external_editor":
					return Promise.resolve("");
				case "detect_editors":
					return Promise.resolve([
						{
							name: "Visual Studio Code",
							path: "/Applications/Visual Studio Code.app",
						},
						{ name: "Cursor", path: "/Applications/Cursor.app" },
					]);
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
				case "get_configured_agents":
					return Promise.resolve([]);
				case "preview_agent_mcp_config":
					return Promise.resolve("");
				case "update_external_editor":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Editor"));

		const trigger = await screen.findByRole("combobox", {
			name: "External Editor",
		});
		await user.click(trigger);
		const option = screen.getByRole("option", { name: "Cursor" });
		await user.click(option);

		const saveBtn = screen.getByRole("button", { name: "Save" });
		expect(saveBtn).toBeEnabled();
		await user.click(saveBtn);

		expect(vi.mocked(invoke)).toHaveBeenCalledWith("update_external_editor", {
			editor: "/Applications/Cursor.app",
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
				case "get_mcp_config":
					return Promise.resolve({ port: 19801, token: "test-token" });
				case "get_configured_agents":
					return Promise.resolve([]);
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

	it("should show workflow list in Automation section", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const emptyReport = {
			items: [],
			workflow_summaries: {},
			facet_summaries: {},
			facet_usage: {},
		};
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([
						{
							name: "quick-fix",
							description: "素早いバグ修正",
							builtin: true,
						},
						{
							name: "my-workflow",
							description: "カスタムワークフロー",
							builtin: false,
						},
					]);
				case "diagnose_all_cmd":
					return Promise.resolve(emptyReport);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Automation"));

		await waitFor(() => {
			expect(screen.getByText("quick-fix")).toBeInTheDocument();
			expect(screen.getByText("my-workflow")).toBeInTheDocument();
		});

		expect(screen.getByText("素早いバグ修正")).toBeInTheDocument();
		expect(screen.getByText("カスタムワークフロー")).toBeInTheDocument();
		expect(screen.getByText("builtin")).toBeInTheDocument();
	});

	it("should show empty state when no workflows exist", async () => {
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Automation"));

		await waitFor(() => {
			expect(
				screen.getByText("Select a workflow to view details"),
			).toBeInTheDocument();
		});
	});

	it("should call open_workflow_in_editor for custom workflow", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		const emptyReport = {
			items: [],
			workflow_summaries: {},
			facet_summaries: {},
			facet_usage: {},
		};
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([
						{
							name: "my-workflow",
							description: "カスタムワークフロー",
							builtin: false,
						},
					]);
				case "open_workflow_in_editor":
					return Promise.resolve(null);
				case "diagnose_all_cmd":
					return Promise.resolve(emptyReport);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Automation"));

		await waitFor(() => {
			expect(screen.getByText("my-workflow")).toBeInTheDocument();
		});

		await user.click(screen.getByTitle("Open in editor"));

		expect(vi.mocked(invoke)).toHaveBeenCalledWith("open_workflow_in_editor", {
			name: "my-workflow",
		});
	});

	it("should call delete_workflow when Delete button is clicked", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		const emptyReport = {
			items: [],
			workflow_summaries: {},
			facet_summaries: {},
			facet_usage: {},
		};
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([
						{
							name: "my-workflow",
							description: "カスタムワークフロー",
							builtin: false,
						},
					]);
				case "delete_workflow":
					return Promise.resolve(null);
				case "diagnose_all_cmd":
					return Promise.resolve(emptyReport);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Automation"));

		await waitFor(() => {
			expect(screen.getByText("my-workflow")).toBeInTheDocument();
		});

		vi.spyOn(window, "confirm").mockReturnValue(true);
		await user.click(screen.getByTitle("Delete"));

		await waitFor(() => {
			expect(vi.mocked(invoke)).toHaveBeenCalledWith("delete_workflow", {
				name: "my-workflow",
			});
		});
	});

	it("should not show delete button for builtin workflows", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const emptyReport = {
			items: [],
			workflow_summaries: {},
			facet_summaries: {},
			facet_usage: {},
		};
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([
						{
							name: "quick-fix",
							description: "素早いバグ修正",
							builtin: true,
						},
					]);
				case "diagnose_all_cmd":
					return Promise.resolve(emptyReport);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Automation"));

		await waitFor(() => {
			expect(screen.getByText("quick-fix")).toBeInTheDocument();
		});

		expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
	});

	describe("Repository removal", () => {
		const repoMockSetup = async () => {
			const { invoke } = await import("@tauri-apps/api/core");
			vi.mocked(invoke).mockImplementation((cmd: string) => {
				switch (cmd) {
					case "list_branches":
						return Promise.resolve([{ name: "main", is_remote: false }]);
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
					case "get_mcp_config":
						return Promise.resolve({ port: 19801, token: "test-token" });
					case "get_configured_agents":
						return Promise.resolve([]);
					case "preview_agent_mcp_config":
						return Promise.resolve("");
					default:
						return Promise.resolve(null);
				}
			});
		};

		it("should show remove button when onRemoveRepo is provided", async () => {
			await repoMockSetup();
			const onRemoveRepo = vi.fn();
			render(<SettingsModal {...defaultProps} onRemoveRepo={onRemoveRepo} />);
			fireEvent.click(screen.getByText("Repositories"));
			await waitFor(() => {
				expect(
					screen.getByRole("button", { name: /Remove repository/ }),
				).toBeInTheDocument();
			});
		});

		it("should not show remove button when onRemoveRepo is not provided", async () => {
			await repoMockSetup();
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Repositories"));
			await waitFor(() => {
				expect(screen.getByText("Base branch")).toBeInTheDocument();
			});
			expect(
				screen.queryByRole("button", { name: /Remove repository/ }),
			).not.toBeInTheDocument();
		});

		it("should show confirm dialog with unregister message when remove button is clicked", async () => {
			await repoMockSetup();
			const user = userEvent.setup();
			const onRemoveRepo = vi.fn();
			render(<SettingsModal {...defaultProps} onRemoveRepo={onRemoveRepo} />);
			fireEvent.click(screen.getByText("Repositories"));
			const removeBtn = await screen.findByRole("button", {
				name: /Remove repository/,
			});
			await user.click(removeBtn);
			expect(
				screen.getByText(
					"Remove from list? The repository will not be deleted from disk.",
				),
			).toBeInTheDocument();
		});

		it("should call onRemoveRepo when deletion is confirmed", async () => {
			await repoMockSetup();
			const user = userEvent.setup();
			const onRemoveRepo = vi.fn();
			render(<SettingsModal {...defaultProps} onRemoveRepo={onRemoveRepo} />);
			fireEvent.click(screen.getByText("Repositories"));
			const removeBtn = await screen.findByRole("button", {
				name: /Remove repository/,
			});
			await user.click(removeBtn);
			await user.click(screen.getByRole("button", { name: "Delete" }));
			expect(onRemoveRepo).toHaveBeenCalledWith("/repos/my-app");
		});

		it("should not call onRemoveRepo when deletion is cancelled", async () => {
			await repoMockSetup();
			const user = userEvent.setup();
			const onRemoveRepo = vi.fn();
			render(<SettingsModal {...defaultProps} onRemoveRepo={onRemoveRepo} />);
			fireEvent.click(screen.getByText("Repositories"));
			const removeBtn = await screen.findByRole("button", {
				name: /Remove repository/,
			});
			await user.click(removeBtn);
			await user.click(screen.getByRole("button", { name: "Cancel" }));
			expect(onRemoveRepo).not.toHaveBeenCalled();
		});
	});
});
