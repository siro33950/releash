import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
	afterEach,
	beforeAll,
	beforeEach,
	describe,
	expect,
	it,
	vi,
} from "vitest";
import { ClientTransportError } from "@/lib/clientSocket";
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
vi.mock("@tauri-apps/plugin-autostart", () => ({
	isEnabled: vi.fn().mockResolvedValue(true),
	enable: vi.fn().mockResolvedValue(undefined),
	disable: vi.fn().mockResolvedValue(undefined),
}));

// Radix UI uses pointer events; jsdom doesn't implement them
beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

describe("SettingsModal", () => {
	afterEach(() => vi.restoreAllMocks());

	beforeEach(async () => {
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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

	it.each(["期限後の応答", "接続回復"])(
		"%sの通知でpushを購読しない設定も取得し直す",
		async () => {
			const { invokeClient, onClientRefresh, ClientTransportError } =
				await import("@/lib/clientSocket");
			const commands = [
				"get_workflow_config",
				"get_external_editor",
				"detect_editors",
				"get_notion_config",
				"get_provider_availability",
				"get_app_settings",
			];
			const callbacks = new Set<() => void>();
			vi.mocked(onClientRefresh).mockImplementation((callback) => {
				callbacks.add(callback);
				return () => {
					callbacks.delete(callback);
				};
			});
			const initial = vi.mocked(invokeClient).getMockImplementation();
			if (!initial) throw new Error("Missing settings fixture");
			let recovered = false;
			vi.mocked(invokeClient).mockImplementation((command, args) => {
				if (commands.includes(command) && !recovered)
					return Promise.reject(new ClientTransportError(command, "unknown"));
				if (command === "get_notion_config")
					return Promise.resolve({
						api_token: "recovered-token",
						database_id: "recovered-db",
						property_mapping: {
							title: "Title",
							labels: [],
							branch_name: "Branch",
							branch_prefix: "fix/",
						},
					});
				if (command === "get_provider_availability")
					return Promise.resolve({
						providers: [
							{
								provider: "codex",
								displayName: "Codex",
								defaultExecutable: "codex",
								configuredExecutable: "/recovered/codex",
								effectiveExecutable: "/recovered/codex",
								available: true,
								resolvedExecutable: "/recovered/codex",
								unavailableReason: null,
							},
						],
					});
				if (command === "get_app_settings")
					return Promise.resolve({
						close_to_tray: false,
						auto_launch: true,
						start_minimized: true,
						last_root_path: "",
						last_repo_paths: [],
						external_editor: "",
					});
				if (command === "get_external_editor")
					return Promise.resolve("/bin/zed");
				if (command === "detect_editors")
					return Promise.resolve([{ name: "Zed", path: "/bin/zed" }]);
				if (command === "get_workflow_config")
					return Promise.resolve({ approval_auto_approve: true });
				return initial(command, args);
			});
			const view = render(<SettingsModal {...defaultProps} />);
			for (const section of ["Editor", "Notion", "Agent", "Background"]) {
				fireEvent.click(screen.getByText(section));
				await waitFor(() =>
					expect(
						screen.getAllByText(/操作結果を確認できません/).length,
					).toBeGreaterThan(0),
				);
				if (section === "Notion") {
					expect(screen.getByRole("alert")).toHaveTextContent("/repos/my-app");
					expect(screen.queryByLabelText("API Token")).not.toBeInTheDocument();
				}
			}
			expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
			await act(async () => {
				recovered = true;
				for (const callback of callbacks) callback();
			});
			await waitFor(() =>
				expect(
					screen.queryByText(/操作結果を確認できません/),
				).not.toBeInTheDocument(),
			);
			expect(
				screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
			).not.toBeChecked();
			expect(
				screen.getByRole("checkbox", { name: "Start minimized" }),
			).toBeChecked();
			fireEvent.click(screen.getByText("Notion"));
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(screen.getByLabelText("API Token")).toHaveValue("recovered-token");
			expect(screen.getByLabelText("Database ID")).toHaveValue("recovered-db");
			fireEvent.click(screen.getByText("Agent"));
			expect(
				screen.queryByText(/操作結果を確認できません/),
			).not.toBeInTheDocument();
			expect(screen.getByLabelText("Codex executable override")).toHaveValue(
				"/recovered/codex",
			);
			expect(screen.queryByText("not_found")).not.toBeInTheDocument();
			fireEvent.click(screen.getByText("Editor"));
			expect(
				screen.queryByText(/操作結果を確認できません/),
			).not.toBeInTheDocument();
			expect(
				screen.getByRole("combobox", { name: "External Editor" }),
			).toHaveTextContent("Zed");
			for (const command of commands)
				expect(
					vi
						.mocked(invokeClient)
						.mock.calls.filter(([name]) => name === command).length,
				).toBeGreaterThanOrEqual(2);
			view.unmount();
			expect(callbacks.size).toBe(0);
			vi.mocked(onClientRefresh).mockImplementation(() => () => {});
		},
	);

	it.each(["editor", "workflow", "base", "background", "notion", "provider"])(
		"再取得しても%sの未保存入力と保存可能な状態を保持する",
		async (form) => {
			const { invokeClient, onClientRefresh } = await import(
				"@/lib/clientSocket"
			);
			const initial = vi.mocked(invokeClient).getMockImplementation();
			if (!initial) throw new Error("Missing invoke fixture");
			const callbacks = new Set<() => void>();
			vi.mocked(onClientRefresh).mockImplementation((callback) => {
				callbacks.add(callback);
				return () => {
					callbacks.delete(callback);
				};
			});
			vi.mocked(invokeClient).mockImplementation((command, args) => {
				if (command === "get_external_editor") return Promise.resolve("code");
				if (command === "detect_editors")
					return Promise.resolve([
						{ name: "Code", path: "code" },
						{ name: "Zed", path: "zed" },
					]);
				if (command === "get_app_settings")
					return Promise.resolve({
						close_to_tray: true,
						auto_launch: true,
						start_minimized: false,
						last_root_path: "",
						last_repo_paths: [],
						external_editor: "",
					});
				if (command === "get_releash_base") return Promise.resolve("main");
				if (command === "list_branches")
					return Promise.resolve([
						{ name: "main", is_remote: false, is_head: true },
						{ name: "develop", is_remote: false, is_head: false },
					]);
				return initial(command, args);
			});
			const user = userEvent.setup();
			const view = render(<SettingsModal {...defaultProps} />);
			const sections = {
				editor: "Editor",
				workflow: "Agent",
				base: "Repositories",
				background: "Background",
				notion: "Notion",
				provider: "Agent",
			};
			await user.click(
				screen.getByText(sections[form as keyof typeof sections]),
			);
			if (form === "editor" || form === "base") {
				await user.click(
					await screen.findByRole("combobox", {
						name: form === "editor" ? "External Editor" : "Base branch",
					}),
				);
				await user.click(
					await screen.findByRole("option", {
						name: form === "editor" ? "Zed" : "develop",
					}),
				);
			} else if (form === "workflow" || form === "background") {
				const checkbox = await screen.findByRole("checkbox", {
					name:
						form === "workflow" ? "Approval auto-approve" : "Start minimized",
				});
				await waitFor(() => expect(checkbox).toBeEnabled());
				await user.click(checkbox);
			} else {
				const input = await screen.findByLabelText(
					form === "notion" ? "API Token" : "Codex executable override",
				);
				await user.clear(input);
				await user.type(input, "unsaved-input");
			}
			expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
			await act(async () => {
				for (const callback of callbacks) callback();
			});
			if (form === "editor" || form === "base") {
				expect(
					await screen.findByRole("combobox", {
						name: form === "editor" ? "External Editor" : "Base branch",
					}),
				).toHaveTextContent(form === "editor" ? "Zed" : "develop");
			} else if (form === "workflow" || form === "background") {
				expect(
					screen.getByRole("checkbox", {
						name:
							form === "workflow" ? "Approval auto-approve" : "Start minimized",
					}),
				).toBeChecked();
			} else {
				expect(
					await screen.findByLabelText(
						form === "notion" ? "API Token" : "Codex executable override",
					),
				).toHaveValue("unsaved-input");
			}
			expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
			view.unmount();
			vi.mocked(onClientRefresh).mockImplementation(() => () => {});
		},
	);

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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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

		expect(invoke).toHaveBeenCalledWith(
			"update_provider_executable",
			{
				provider: "claude",
				executable: "/custom/bin/claude",
			},
			{ onUncertain: expect.any(Function) },
		);
	});

	it("Provider CLIのresetとrefreshをbackend操作へ転送する", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		await user.click(
			await screen.findByRole("button", { name: "Reset Claude executable" }),
		);
		await user.click(
			screen.getByRole("button", { name: "Refresh Provider CLI availability" }),
		);

		expect(invoke).toHaveBeenCalledWith(
			"reset_provider_executable",
			{
				provider: "claude",
			},
			{ onUncertain: expect.any(Function) },
		);
		expect(invoke).toHaveBeenCalledWith(
			"refresh_provider_availability",
			undefined,
			{ onUncertain: expect.any(Function) },
		);
	});

	it("一方のProvider CLIをresetしても他方の未保存draftを維持する", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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

	it("should load and save approval auto-approve independently from agent auto-approve", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
			name: "Approval auto-approve",
		});
		expect(
			screen.getByText(
				"Automatically approves completed nodes with completion.require: approval.",
			),
		).toBeInTheDocument();
		await waitFor(() => {
			expect(workflowCheckbox).toBeChecked();
		});

		await user.click(workflowCheckbox);
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(invoke).toHaveBeenCalledWith(
			"update_workflow_config",
			{
				workflow: { approval_auto_approve: false },
			},
			{ onUncertain: expect.any(Function) },
		);
	});

	it("should save external editor selection via Save button", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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

		expect(vi.mocked(invoke)).toHaveBeenCalledWith(
			"update_external_editor",
			{
				editor: "/Applications/Cursor.app",
			},
			{ onUncertain: expect.any(Function) },
		);
	});

	it("should save base branch via Apply button", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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

		expect(vi.mocked(invoke)).toHaveBeenCalledWith(
			"set_releash_base",
			{
				repoPath: "/repos/my-app",
				base: "develop",
			},
			{ onUncertain: expect.any(Function) },
		);
	});

	it("should show workflow list in Automation section", async () => {
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
							sourceFormat: "yaml" as const,
							is_running: false,
						},
						{
							name: "my-workflow",
							description: "カスタムワークフロー",
							builtin: false,
							sourceFormat: "yaml" as const,
							is_running: false,
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
							sourceFormat: "yaml" as const,
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
						sourceFormat: "yaml",
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
							sourceFormat: "yaml" as const,
							is_running: false,
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
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
							sourceFormat: "yaml" as const,
							is_running: false,
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
			const { invokeClient: invoke } = await import("@/lib/clientSocket");
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
	it("回復時には未保存入力を保ち、閉じて開き直すと各設定の現在値に戻す", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke, onClientRefresh } = await import(
			"@/lib/clientSocket"
		);
		const base = vi.mocked(invoke).getMockImplementation();
		if (!base) throw new Error("Missing invoke fixture");
		vi.mocked(invoke).mockImplementation((command, args) => {
			if (command === "get_external_editor") return Promise.resolve("code");
			if (command === "detect_editors")
				return Promise.resolve([
					{ name: "Code", path: "code" },
					{ name: "Zed", path: "zed" },
				]);
			return base(command, args);
		});
		const view = render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		const approval = await screen.findByRole("checkbox", {
			name: "Approval auto-approve",
		});
		await user.click(approval);
		const cli = await screen.findByDisplayValue("/opt/custom/claude");
		fireEvent.change(cli, { target: { value: "/draft/claude" } });
		fireEvent.click(screen.getByText("Editor"));
		await user.click(
			await screen.findByRole("combobox", { name: "External Editor" }),
		);
		await user.click(screen.getByRole("option", { name: "Zed" }));
		await act(async () => {
			for (const [refresh] of vi.mocked(onClientRefresh).mock.calls) refresh();
		});
		expect(
			await screen.findByRole("combobox", { name: "External Editor" }),
		).toHaveTextContent("Zed");
		fireEvent.click(screen.getByText("Agent"));
		expect(
			await screen.findByRole("checkbox", { name: "Approval auto-approve" }),
		).toBeChecked();
		expect(screen.getByDisplayValue("/draft/claude")).toBeInTheDocument();
		view.rerender(<SettingsModal {...defaultProps} open={false} />);
		view.rerender(<SettingsModal {...defaultProps} open />);
		fireEvent.click(screen.getByText("Agent"));
		await waitFor(() =>
			expect(
				screen.getByRole("checkbox", { name: "Approval auto-approve" }),
			).not.toBeChecked(),
		);
		expect(
			await screen.findByDisplayValue("/opt/custom/claude"),
		).toBeInTheDocument();
		expect(screen.queryByDisplayValue("/draft/claude")).not.toBeInTheDocument();
		fireEvent.click(screen.getByText("Editor"));
		expect(
			await screen.findByRole("combobox", { name: "External Editor" }),
		).toHaveTextContent("Code");
	});
	it.each(["success", "failure"])(
		"workflow保存の結果不明を表示し遅延%sを反映する",
		async (outcome) => {
			const client = await import("@/lib/clientSocket");
			const { invokeClient: invoke } = client;
			const retry = vi
				.spyOn(client, "retryClientOperation")
				.mockImplementation(() => {});
			const base = vi.mocked(invoke).getMockImplementation();
			if (!base) throw new Error("Missing invoke fixture");
			vi.mocked(invoke).mockClear();
			let options: Parameters<typeof invoke>[2];
			let complete!: () => void;
			let fail!: (error: Error) => void;
			vi.mocked(invoke).mockImplementation((command, args, nextOptions) => {
				if (command !== "update_workflow_config") return base(command, args);
				options = nextOptions;
				return new Promise<void>((resolve, reject) => {
					complete = resolve;
					fail = reject;
				});
			});
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Agent"));
			const checkbox = await screen.findByRole("checkbox", {
				name: "Approval auto-approve",
			});
			await userEvent.click(checkbox);
			await userEvent.click(screen.getByRole("button", { name: "Save" }));
			expect(options?.onUncertain).toEqual(expect.any(Function));
			act(() =>
				options?.onUncertain?.(new ClientTransportError("save", "unknown")),
			);
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			const save = screen.getByRole("button", {
				name: "元の操作の結果を確認",
			});
			expect(save).toBeEnabled();
			expect(save.querySelector(".animate-spin")).toBeNull();
			await userEvent.click(save);
			expect(retry).toHaveBeenCalledExactlyOnceWith("save");
			expect(
				vi
					.mocked(invoke)
					.mock.calls.filter(
						([command]) => command === "update_workflow_config",
					),
			).toHaveLength(1);
			await act(async () => {
				if (outcome === "success") complete();
				else fail(new Error("保存が拒否されました"));
			});
			if (outcome === "success") {
				expect(screen.queryByRole("alert")).not.toBeInTheDocument();
				expect(save).toBeDisabled();
				expect(checkbox).toBeChecked();
			} else {
				expect(screen.getByRole("alert")).toHaveTextContent(
					"保存が拒否されました",
				);
				expect(save).toBeEnabled();
			}
		},
	);

	it.each(["success", "failure"])(
		"背景設定保存の結果不明を表示し遅延%sを反映する",
		async (outcome) => {
			const client = await import("@/lib/clientSocket");
			const { invokeClient: invoke, onClientRefresh } = client;
			const retry = vi
				.spyOn(client, "retryClientOperation")
				.mockImplementation(() => {});
			const base = vi.mocked(invoke).getMockImplementation();
			if (!base) throw new Error("Missing invoke fixture");
			vi.mocked(invoke).mockClear();
			let options: Parameters<typeof invoke>[2];
			let complete!: () => void;
			let fail!: (error: Error) => void;
			vi.mocked(invoke).mockImplementation((command, args, nextOptions) => {
				if (command === "get_app_settings")
					return Promise.resolve({
						close_to_tray: true,
						auto_launch: true,
						start_minimized: false,
						last_root_path: "",
						last_repo_paths: [],
						external_editor: "",
					});
				if (command !== "update_app_settings") return base(command, args);
				options = nextOptions;
				return new Promise<void>((resolve, reject) => {
					complete = resolve;
					fail = reject;
				});
			});
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Background"));
			const checkbox = await screen.findByRole("checkbox", {
				name: "Minimize to tray on close",
			});
			await userEvent.click(checkbox);
			const save = screen.getByRole("button", { name: "Save" });
			await userEvent.click(save);
			expect(save.querySelector(".animate-spin")).not.toBeNull();
			expect(options?.onUncertain).toEqual(expect.any(Function));
			act(() =>
				options?.onUncertain?.(new ClientTransportError("save", "unknown")),
			);
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			expect(save).toHaveTextContent("元の操作の結果を確認");
			expect(save.querySelector(".animate-spin")).toBeNull();
			expect(save).toBeEnabled();
			fireEvent.click(screen.getByText("Appearance"));
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			fireEvent.click(screen.getByText("Background"));
			await act(async () => {
				for (const [refresh] of vi.mocked(onClientRefresh).mock.calls)
					refresh();
			});
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			expect(
				screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
			).not.toBeChecked();
			await userEvent.click(save);
			expect(retry).toHaveBeenCalledExactlyOnceWith("save");
			expect(
				vi
					.mocked(invoke)
					.mock.calls.filter(([command]) => command === "update_app_settings"),
			).toHaveLength(1);
			await act(async () => {
				if (outcome === "success") complete();
				else fail(new Error("保存が拒否されました"));
			});
			if (outcome === "success") {
				expect(screen.queryByRole("alert")).not.toBeInTheDocument();
				expect(save).toBeDisabled();
				expect(
					screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
				).not.toBeChecked();
			} else {
				expect(screen.getByRole("alert")).toHaveTextContent(
					"保存が拒否されました",
				);
				expect(save).toBeEnabled();
			}
		},
	);

	it.each(
		[
			"update_external_editor",
			"set_releash_base",
			"save_notion_config",
			"delete_notion_config",
			"update_provider_executable",
			"reset_provider_executable",
			"refresh_provider_availability",
		].flatMap((command) =>
			[false, true].map((failure) => [command, failure] as const),
		),
	)(
		"%sの結果不明で待機を解除し元の操作の遅延結果を表示する: failure=%s",
		async (command, failure) => {
			const client = await import("@/lib/clientSocket");
			const invoke = vi.mocked(client.invokeClient);
			const base = invoke.getMockImplementation();
			invoke.mockClear();
			if (!base) throw new Error("Missing settings fixture");
			const retry = vi
				.spyOn(client, "retryClientOperation")
				.mockImplementation(() => {});
			let options: Parameters<typeof client.invokeClient>[2];
			let complete!: (
				value: Awaited<ReturnType<typeof client.invokeClient>>,
			) => void;
			let fail!: (error: Error) => void;
			let confirmed = false;
			const providerSnapshot = (await base(
				"get_provider_availability",
			)) as Awaited<
				ReturnType<typeof client.invokeClient<"get_provider_availability">>
			>;
			const nextProviders = {
				providers: providerSnapshot.providers.map((provider) =>
					provider.provider === "claude"
						? {
								...provider,
								configuredExecutable:
									command === "reset_provider_executable"
										? null
										: "/saved/claude",
								effectiveExecutable:
									command === "reset_provider_executable"
										? "claude"
										: "/saved/claude",
							}
						: provider,
				),
			};
			const config = {
				api_token: "token",
				database_id: "db",
				property_mapping: {
					title: "Name",
					labels: [],
					branch_name: "",
					branch_prefix: "",
				},
			};
			invoke.mockImplementation((name, args, requestOptions) => {
				if (name === command) {
					options = requestOptions;
					return new Promise((resolve, reject) => {
						complete = resolve;
						fail = reject;
					});
				}
				if (name === "detect_editors")
					return Promise.resolve([
						{ name: "Code", path: "code" },
						{ name: "Zed", path: "zed" },
					]);
				if (name === "get_external_editor")
					return Promise.resolve(confirmed ? "zed" : "code");
				if (name === "list_branches")
					return Promise.resolve([
						{ name: "main", is_remote: false },
						{ name: "develop", is_remote: false },
					]);
				if (name === "get_releash_base")
					return Promise.resolve(confirmed ? "develop" : "main");
				if (name === "get_notion_config")
					return Promise.resolve(
						confirmed
							? command === "delete_notion_config"
								? null
								: { ...config, api_token: "saved" }
							: config,
					);
				if (name === "get_provider_availability")
					return Promise.resolve(confirmed ? nextProviders : providerSnapshot);
				return base(name, args, requestOptions);
			});
			render(<SettingsModal {...defaultProps} />);
			const section =
				command === "update_external_editor"
					? "Editor"
					: command === "set_releash_base"
						? "Repositories"
						: command.includes("notion")
							? "Notion"
							: "Agent";
			fireEvent.click(screen.getByText(section));
			if (
				command === "update_external_editor" ||
				command === "set_releash_base"
			) {
				await userEvent.click(
					await screen.findByRole("combobox", {
						name:
							command === "update_external_editor"
								? "External Editor"
								: "Base branch",
					}),
				);
				await userEvent.click(
					screen.getByRole("option", {
						name: command === "update_external_editor" ? "Zed" : "develop",
					}),
				);
			} else if (command === "save_notion_config") {
				fireEvent.change(await screen.findByLabelText("API Token"), {
					target: { value: "saved" },
				});
			} else if (command === "delete_notion_config") {
				await userEvent.click(
					await screen.findByRole("button", {
						name: "Delete Notion configuration",
					}),
				);
			} else if (command === "update_provider_executable") {
				fireEvent.change(
					await screen.findByLabelText("Claude executable override"),
					{ target: { value: "/saved/claude" } },
				);
			}
			const action =
				command === "reset_provider_executable"
					? "Reset Claude executable"
					: command === "refresh_provider_availability"
						? "Refresh Provider CLI availability"
						: "Save";
			const button = await screen.findByRole("button", { name: action });
			await userEvent.click(button);
			expect(button).toBeDisabled();
			expect(options?.onUncertain).toEqual(expect.any(Function));
			act(() =>
				options?.onUncertain?.(
					new client.ClientTransportError("original", "unknown"),
				),
			);
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			expect(button).toBeEnabled();
			expect(button.querySelector(".animate-spin")).toBeNull();
			expect(button).toHaveTextContent("元の操作の結果を確認");
			await userEvent.click(button);
			expect(retry).toHaveBeenCalledExactlyOnceWith("original");
			if (section === "Agent")
				expect(
					screen.getByLabelText("Claude executable override"),
				).toBeEnabled();
			fireEvent.click(screen.getByText("Appearance"));
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			fireEvent.click(screen.getByText(section));
			const confirms = screen.getAllByRole("button", {
				name: "元の操作の結果を確認",
			});
			await userEvent.click(confirms[confirms.length - 1]);
			expect(retry).toHaveBeenCalledTimes(2);
			expect(retry).toHaveBeenLastCalledWith("original");
			expect(
				invoke.mock.calls.filter(([name]) => name === command),
			).toHaveLength(1);
			await act(async () => {
				if (failure) fail(new Error("変更が拒否されました"));
				else {
					confirmed = true;
					complete(section === "Agent" ? nextProviders : null);
				}
			});
			if (failure) {
				expect(screen.getByText("変更が拒否されました")).toBeInTheDocument();
				expect(
					screen.queryByText(/操作結果を確認できません/),
				).not.toBeInTheDocument();
				return;
			}
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
			if (command === "update_external_editor")
				expect(
					screen.getByRole("combobox", { name: "External Editor" }),
				).toHaveTextContent("Zed");
			if (command === "set_releash_base")
				expect(
					await screen.findByRole("combobox", { name: "Base branch" }),
				).toHaveTextContent("develop");
			if (command === "save_notion_config")
				expect(screen.getByLabelText("API Token")).toHaveValue("saved");
			if (command === "delete_notion_config")
				expect(screen.getByLabelText("API Token")).toHaveValue("");
			if (section === "Agent")
				expect(screen.getByLabelText("Claude executable override")).toHaveValue(
					command === "reset_provider_executable" ? "" : "/saved/claude",
				);
		},
	);
	it.each([
		"update_provider_executable",
		"reset_provider_executable",
		"refresh_provider_availability",
	] as const)(
		"%sの結果不明と別providerのReset・Refreshを操作ごとに表示する",
		async (command) => {
			const client = await import("@/lib/clientSocket");
			const invoke = vi.mocked(client.invokeClient);
			const base = invoke.getMockImplementation();
			if (!base) throw new Error("Missing settings fixture");
			const snapshot = (await base("get_provider_availability")) as Awaited<
				ReturnType<typeof client.invokeClient<"get_provider_availability">>
			>;
			for (const provider of snapshot.providers)
				provider.configuredExecutable = `/custom/${provider.provider}`;
			const pending = new Map<
				string,
				{
					options: Parameters<typeof client.invokeClient>[2];
					resolve: (value: typeof snapshot) => void;
				}
			>();
			const retry = vi
				.spyOn(client, "retryClientOperation")
				.mockImplementation(() => {});
			invoke.mockImplementation((name, args, options) => {
				if (name === "get_provider_availability")
					return Promise.resolve(snapshot);
				if (
					[
						"update_provider_executable",
						"reset_provider_executable",
						"refresh_provider_availability",
					].includes(name)
				) {
					const provider = args && "provider" in args ? args.provider : "";
					return new Promise((resolve) =>
						pending.set(`${name}:${provider}`, { options, resolve }),
					);
				}
				return base(name, args, options);
			});
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Agent"));
			await screen.findByDisplayValue("/custom/claude");
			if (command === "update_provider_executable") {
				fireEvent.change(screen.getByLabelText("Claude executable override"), {
					target: { value: "/saved/claude" },
				});
			}
			const originalId = `${command}:${command === "refresh_provider_availability" ? "" : "claude"}`;
			const original = screen.getByRole("button", {
				name:
					command === "update_provider_executable"
						? "Save"
						: command === "reset_provider_executable"
							? "Reset Claude executable"
							: "Refresh Provider CLI availability",
			});
			await userEvent.click(original);
			act(() =>
				pending
					.get(originalId)
					?.options?.onUncertain?.(
						new ClientTransportError(originalId, "unknown"),
					),
			);
			const codex = screen.getByRole("button", {
				name: "Reset Codex executable",
			});
			await userEvent.click(codex);
			const codexId = "reset_provider_executable:codex";
			expect(pending.has(codexId)).toBe(true);
			act(() =>
				pending
					.get(codexId)
					?.options?.onUncertain?.(
						new ClientTransportError(codexId, "unknown"),
					),
			);
			await userEvent.click(codex);
			expect(retry).toHaveBeenLastCalledWith(codexId);
			const otherId =
				command === "refresh_provider_availability"
					? "reset_provider_executable:claude"
					: "refresh_provider_availability:";
			await userEvent.click(
				screen.getByRole("button", {
					name:
						command === "refresh_provider_availability"
							? "Reset Claude executable"
							: "Refresh Provider CLI availability",
				}),
			);
			expect(pending.has(otherId)).toBe(true);
			await act(async () => pending.get(otherId)?.resolve(snapshot));
			expect(codex).toHaveTextContent("元の操作の結果を確認");
			await userEvent.click(codex);
			expect(retry).toHaveBeenLastCalledWith(codexId);
			await act(async () => pending.get(codexId)?.resolve(snapshot));
			expect(codex).toHaveTextContent("Reset");
			expect(original).toHaveTextContent("元の操作の結果を確認");
			await userEvent.click(original);
			expect(retry).toHaveBeenLastCalledWith(originalId);
			expect(pending.size).toBe(3);
			await act(async () => pending.get(originalId)?.resolve(snapshot));
			expect(
				screen.queryByText(/操作結果を確認できません/),
			).not.toBeInTheDocument();
		},
	);

	it("providerフォームを開き直すと取得中と取得失敗時にも以前のdraftを保存しない", async () => {
		const { invokeClient: invoke } = await import("@/lib/clientSocket");
		const base = vi.mocked(invoke).getMockImplementation();
		if (!base) throw new Error("Missing invoke fixture");
		vi.mocked(invoke).mockClear();
		const view = render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		fireEvent.change(await screen.findByDisplayValue("/opt/custom/claude"), {
			target: { value: "/unsaved/claude" },
		});
		expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
		view.rerender(<SettingsModal {...defaultProps} open={false} />);
		let fail!: (error: Error) => void;
		vi.mocked(invoke).mockImplementation((command, args) =>
			command === "get_provider_availability"
				? new Promise((_, reject) => {
						fail = reject;
					})
				: base(command, args),
		);
		view.rerender(<SettingsModal {...defaultProps} open />);
		fireEvent.click(screen.getByText("Agent"));
		expect(
			screen.queryByDisplayValue("/unsaved/claude"),
		).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
		await act(async () => fail(new Error("取得できません")));
		expect(screen.getByRole("alert")).toHaveTextContent("取得できません");
		expect(
			screen.queryByDisplayValue("/unsaved/claude"),
		).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
		await userEvent.click(
			screen.getByRole("checkbox", { name: "Approval auto-approve" }),
		);
		await userEvent.click(screen.getByRole("button", { name: "Save" }));
		expect(
			vi
				.mocked(invoke)
				.mock.calls.some(
					([command]) => command === "update_provider_executable",
				),
		).toBe(false);
	});
});
