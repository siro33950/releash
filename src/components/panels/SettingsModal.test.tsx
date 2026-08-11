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

const monacoMock = vi.hoisted(() => {
	const model = {
		getValue: vi.fn(() => "name: my-workflow\nnodes: []\n"),
		dispose: vi.fn(),
	};
	const editor = {
		dispose: vi.fn(),
		onDidChangeModelContent: vi.fn(() => ({ dispose: vi.fn() })),
	};
	return {
		module: {
			MarkerSeverity: { Error: 8, Warning: 4, Info: 2 },
			editor: {
				createModel: vi.fn(() => model),
				create: vi.fn(() => editor),
				setModelMarkers: vi.fn(),
			},
		},
	};
});

vi.mock("monaco-editor", () => monacoMock.module);

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
				case "get_workflow_config":
					return Promise.resolve({
						approval_auto_approve: false,
					});
				case "get_provider_availability":
					return Promise.resolve({
						providers: [
							{
								provider: "claude",
								displayName: "Claude",
								defaultExecutable: "claude",
								configuredExecutable: "/opt/custom/claude",
								effectiveExecutable: "/opt/custom/claude",
								available: true,
								resolvedExecutable: "/opt/custom/claude",
								unavailableReason: null,
							},
							{
								provider: "codex",
								displayName: "Codex",
								defaultExecutable: "codex",
								configuredExecutable: null,
								effectiveExecutable: "codex",
								available: false,
								resolvedExecutable: null,
								unavailableReason: "not_found",
							},
						],
					});
				case "update_workflow_config":
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

	it("does not expose the retired agent command palette settings", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));

		expect(screen.queryByText("Agent shortcuts")).not.toBeInTheDocument();
		expect(screen.queryByLabelText(/Command menu/)).not.toBeInTheDocument();
		expect(
			vi
				.mocked(invoke)
				.mock.calls.some(([command]) =>
					String(command).includes("agent_shortcut"),
				),
		).toBe(false);
	});

	it("does not expose or invoke the legacy Claude Hook configuration", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));

		expect(screen.queryByText("Claude Code Hooks")).not.toBeInTheDocument();
		for (const removed of [
			"generate_hooks_config",
			"get_hooks_status",
			"apply_hooks_config",
		]) {
			expect(
				vi.mocked(invoke).mock.calls.some(([command]) => command === removed),
			).toBe(false);
		}
	});

	it("Provider CLIはbackend一覧から利用可能と利用不可を表示する", async () => {
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));

		expect(
			await screen.findByText("Provider CLI availability"),
		).toBeInTheDocument();
		expect(await screen.findByText("Claude")).toBeInTheDocument();
		expect(screen.getAllByText("/opt/custom/claude").length).toBeGreaterThan(0);
		expect(screen.getByText("not_found")).toBeInTheDocument();
	});

	it("Provider CLIはprovider IDと既定commandを明示する", async () => {
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));

		await screen.findByText("Provider CLI availability");
		expect(await screen.findAllByText("Provider ID")).toHaveLength(2);
		expect(screen.getAllByText("Default")).toHaveLength(2);
		expect(screen.getAllByText("claude", { selector: "span" })).toHaveLength(2);
		expect(screen.getAllByText("codex", { selector: "span" })).toHaveLength(3);
	});

	it("Provider CLI path変更をglobal Saveからbackendへ保存する", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "get_provider_availability") {
				return Promise.resolve({
					providers: [
						{
							provider: "claude",
							displayName: "Claude",
							defaultExecutable: "claude",
							configuredExecutable: null,
							effectiveExecutable: "claude",
							available: true,
							resolvedExecutable: "/usr/bin/claude",
							unavailableReason: null,
						},
					],
				});
			}
			if (cmd === "update_provider_executable") {
				return Promise.resolve({ providers: [] });
			}
			return Promise.resolve(null);
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		const input = await screen.findByLabelText("Claude executable override");
		await user.clear(input);
		await user.type(input, "/custom/bin/claude");
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(invoke).toHaveBeenCalledWith("update_provider_executable", {
			provider: "claude",
			executable: "/custom/bin/claude",
		});
	});

	it("Provider CLIのresetとrefreshをbackend操作へ転送する", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		await user.click(
			await screen.findByRole("button", { name: "Reset Claude executable" }),
		);
		await user.click(
			screen.getByRole("button", { name: "Refresh Provider CLI availability" }),
		);

		expect(invoke).toHaveBeenCalledWith("reset_provider_executable", {
			provider: "claude",
		});
		expect(invoke).toHaveBeenCalledWith("refresh_provider_availability");
	});

	it("一方のProvider CLIをresetしても他方の未保存draftを維持する", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		const provider = (id: string, configuredExecutable: string | null) => ({
			provider: id,
			displayName: id === "claude" ? "Claude" : "Codex",
			defaultExecutable: id,
			configuredExecutable,
			effectiveExecutable: configuredExecutable ?? id,
			available: true,
			resolvedExecutable: `/usr/bin/${id}`,
			unavailableReason: null,
		});
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "get_provider_availability") {
				return Promise.resolve({
					providers: [
						provider("claude", "/opt/custom/claude"),
						provider("codex", null),
					],
				});
			}
			if (command === "reset_provider_executable") {
				return Promise.resolve({
					providers: [provider("claude", null), provider("codex", null)],
				});
			}
			return Promise.resolve(null);
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		const codex = await screen.findByLabelText("Codex executable override");
		await user.type(codex, "/draft/codex");

		await user.click(
			screen.getByRole("button", { name: "Reset Claude executable" }),
		);

		expect(codex).toHaveValue("/draft/codex");
	});

	it("Provider CLI refresh失敗時は直前snapshotを維持してerrorを表示する", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "get_provider_availability") {
				return Promise.resolve({
					providers: [
						{
							provider: "dynamic-provider",
							displayName: "Dynamic Provider",
							defaultExecutable: "dynamic",
							configuredExecutable: null,
							effectiveExecutable: "dynamic",
							available: true,
							resolvedExecutable: "/bin/dynamic",
							unavailableReason: null,
						},
					],
				});
			}
			if (cmd === "refresh_provider_availability") {
				return Promise.reject(new Error("refresh failed"));
			}
			return Promise.resolve(null);
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		expect(await screen.findByText("Dynamic Provider")).toBeInTheDocument();

		await user.click(
			screen.getByRole("button", {
				name: "Refresh Provider CLI availability",
			}),
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"refresh failed",
		);
		expect(screen.getByText("Dynamic Provider")).toBeInTheDocument();
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
		expect(
			screen.getByText("Send anonymous performance metrics"),
		).toBeInTheDocument();
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

	it("should toggle performance telemetry off and call onSave", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invoke } = await import("@tauri-apps/api/core");
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", {
			name: "Send anonymous performance metrics",
		});
		await user.click(checkbox);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith(
			expect.objectContaining({ performanceTelemetry: false }),
		);
		expect(invoke).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: false,
		});
	});

	it("should re-enable performance telemetry and call onSave", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invoke } = await import("@tauri-apps/api/core");
		render(
			<SettingsModal
				{...defaultProps}
				settings={{ ...defaultSettings, performanceTelemetry: false }}
				onSave={onSave}
			/>,
		);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", {
			name: "Send anonymous performance metrics",
		});
		await user.click(checkbox);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith(
			expect.objectContaining({ performanceTelemetry: true }),
		);
		expect(invoke).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: true,
		});
	});

	it("should call settings_saved after performance telemetry update completes", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invoke } = await import("@tauri-apps/api/core");
		const callOrder: string[] = [];
		let resolveTelemetryUpdate: (() => void) | undefined;

		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		await user.click(
			screen.getByRole("checkbox", {
				name: "Send anonymous performance metrics",
			}),
		);
		vi.clearAllMocks();
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "update_performance_telemetry") {
				callOrder.push("update_performance_telemetry:start");
				return new Promise((resolve) => {
					resolveTelemetryUpdate = () => {
						callOrder.push("update_performance_telemetry:done");
						resolve(null);
					};
				});
			}
			if (cmd === "report_usage_event") {
				callOrder.push("settings_saved");
				return Promise.resolve(null);
			}
			return Promise.resolve(null);
		});
		await user.click(screen.getByRole("button", { name: "Save" }));

		await waitFor(() => {
			expect(callOrder).toEqual(["update_performance_telemetry:start"]);
		});
		expect(invoke).not.toHaveBeenCalledWith("report_usage_event", {
			name: "settings_saved",
		});

		resolveTelemetryUpdate?.();

		await waitFor(() => {
			expect(callOrder).toEqual([
				"update_performance_telemetry:start",
				"update_performance_telemetry:done",
				"settings_saved",
			]);
		});
		expect(invoke).toHaveBeenCalledWith("report_usage_event", {
			name: "settings_saved",
		});
	});

	it("does not expose the removed Notifications settings", () => {
		render(<SettingsModal {...defaultProps} />);
		expect(screen.queryByText("Notifications")).not.toBeInTheDocument();
		expect(screen.queryByLabelText("Webhook URL")).not.toBeInTheDocument();
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
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Repositories")).toBeInTheDocument();
		fireEvent.click(screen.getByText("Repositories"));
		expect(await screen.findByText("Base branch")).toBeInTheDocument();
	});

	it("should load and save approval gate auto-approve independently from agent auto-approve", async () => {
		const user = userEvent.setup();
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "get_workflow_config":
					return Promise.resolve({
						approval_auto_approve: true,
					});
				case "update_workflow_config":
					return Promise.resolve(null);
				default:
					return Promise.resolve(null);
			}
		});

		render(<SettingsModal {...defaultProps} />);
		const nav = screen.getByRole("navigation");
		fireEvent.click(within(nav).getByText("Agent"));
		const workflowCheckbox = await screen.findByRole("checkbox", {
			name: "Approval gate auto-approve",
		});
		await waitFor(() => {
			expect(workflowCheckbox).toBeChecked();
		});

		await user.click(workflowCheckbox);
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(invoke).toHaveBeenCalledWith("update_workflow_config", {
			workflow: { approval_auto_approve: false },
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

	it("should open custom workflow in the panel editor", async () => {
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
							is_running: false,
						},
					]);
				case "get_workflow_source":
					return Promise.resolve("name: my-workflow\nnodes: []\n");
				case "get_workflow":
					return Promise.resolve({
						name: "my-workflow",
						description: "カスタムワークフロー",
						builtin: false,
						nodes: [],
					});
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

		await user.click(screen.getByTitle("Edit"));

		await waitFor(() => {
			expect(screen.getByText("Workflow YAML")).toBeInTheDocument();
		});
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("get_workflow_source", {
			name: "my-workflow",
		});
		expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
			"open_workflow_in_editor",
			expect.anything(),
		);
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
