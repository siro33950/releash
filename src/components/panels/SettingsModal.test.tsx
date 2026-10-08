import { Code, ConnectError } from "@connectrpc/connect";
import { invoke as invokeTauri } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
	within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Profiler } from "react";
import {
	afterEach,
	beforeAll,
	beforeEach,
	describe,
	expect,
	it,
	vi,
} from "vitest";
import { useSettings } from "@/hooks/useSettings";
import {
	type StateTarget,
	type StateValues,
	subscribeState,
} from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { type AppSettings, DEFAULT_SETTINGS } from "@/types/settings";
import { SettingsModal } from "./SettingsModal";

const states = stateSubscriptions();
const REPO = "/repos/my-app";
const providerSnapshot = {
	providers: [
		{
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: "/opt/custom/claude",
			configurationRevision: 0,
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
			configurationRevision: 0,
			effectiveExecutable: "codex",
			available: false,
			resolvedExecutable: null,
			unavailableReason: "not_found",
		},
	],
};
const desktopSettings = {
	closeToTray: true,
	startMinimized: false,
	crashReporting: true,
	performanceTelemetry: true,
	autoLaunch: false,
};
const editors = [
	{ name: "Code", path: "code" },
	{ name: "Zed", path: "zed" },
];
const EMPTY_REPORT = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};
function subscribeStates(values: Record<string, unknown>) {
	for (const [target, value] of Object.entries(values)) {
		const [kind, ...args] = target.split(":");
		states.publish(
			(args.length ? { kind, args } : kind) as never,
			value as never,
		);
	}
}
function publishDefaults({ provider = true, desktop = true } = {}) {
	states.clear();
	subscribeStates({ workflows: [], diagnostics: EMPTY_REPORT });
	states.publish({ kind: "branches", args: [REPO] }, [
		{ name: "main", is_remote: false },
		{ name: "develop", is_remote: false },
	]);
	states.publish("workflow-config", { approval_auto_approve: false });
	if (provider) states.publish("provider-availability", providerSnapshot);
	states.publish("external-editor", { selected: "", editors: [] });
	if (desktop) states.publish("desktop-settings", desktopSettings);
	states.publish({ kind: "releash-base", args: [REPO] }, null);
	states.publish({ kind: "notion-config", args: [REPO] }, null);
}

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
	afterEach(() => vi.restoreAllMocks());

	beforeEach(async () => {
		vi.mocked(subscribeState).mockImplementation(((
			...args: Parameters<typeof states.subscribeState>
		) => states.subscribeState(...args)) as typeof subscribeState);
		publishDefaults();
		vi.mocked(invokeTauri).mockResolvedValue({
			enabled: false,
			requiresApproval: false,
			reason: null,
		});
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "update_workflow_config":
					return Promise.resolve(null as never);
				default:
					return Promise.resolve(null as never);
			}
		});
	});

	const defaultSettings: AppSettings = { ...DEFAULT_SETTINGS };
	const { performanceTelemetry: _performanceTelemetry, ...defaultDraft } =
		defaultSettings;

	const defaultProps = {
		open: true,
		onOpenChange: vi.fn(),
		settings: defaultSettings,
		desktopSettingsLoaded: true,
		desktopSettingsError: null,
		onSave: vi.fn(),
		repoPaths: ["/repos/my-app"],
	};

	it.each([null, "desktop settings unavailable"])(
		"設定未取得ならmetricsを保存せず失敗を表示する: %s",
		async (loadError) => {
			const user = userEvent.setup();
			const onSave = vi.fn();
			const { invokeClient } = await import("@/lib/client");
			render(
				<SettingsModal
					{...defaultProps}
					desktopSettingsLoaded={false}
					desktopSettingsError={loadError}
					onSave={onSave}
				/>,
			);
			fireEvent.click(screen.getByText("Privacy & Updates"));
			const metrics = screen.getByRole("checkbox", {
				name: "Send anonymous performance metrics",
			});
			expect(metrics).toBeDisabled();
			await user.click(metrics);
			expect(metrics).toHaveAttribute(
				"data-state",
				defaultSettings.performanceTelemetry ? "checked" : "unchecked",
			);
			if (loadError)
				expect(screen.getByRole("alert")).toHaveTextContent(loadError);
			await user.click(screen.getByRole("checkbox", { name: "Auto-update" }));
			vi.mocked(invokeClient).mockClear();
			await user.click(screen.getByRole("button", { name: "Save" }));
			await waitFor(() =>
				expect(onSave).toHaveBeenCalledWith({
					...defaultDraft,
					autoUpdate: false,
				}),
			);
			expect(
				vi
					.mocked(invokeClient)
					.mock.calls.some(
						([command]) => command === "update_performance_telemetry",
					),
			).toBe(false);
			vi.mocked(invokeClient).mockClear();
		},
	);

	it("設定画面を開いたままmetricsを取得しても他欄の保存で初期値を上書きしない", async () => {
		publishDefaults({ desktop: false });
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invokeClient } = await import("@/lib/client");
		const view = render(
			<SettingsModal
				{...defaultProps}
				desktopSettingsLoaded={false}
				onSave={onSave}
			/>,
		);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const metrics = screen.getByRole("checkbox", {
			name: "Send anonymous performance metrics",
		});
		expect(metrics).toBeDisabled();
		await user.click(screen.getByRole("checkbox", { name: "Auto-update" }));
		await act(async () =>
			states.publish("desktop-settings", {
				...desktopSettings,
				performanceTelemetry: false,
			}),
		);
		view.rerender(
			<SettingsModal
				{...defaultProps}
				settings={{ ...defaultSettings, performanceTelemetry: false }}
				onSave={onSave}
			/>,
		);
		expect(metrics).toBeEnabled();
		expect(metrics).not.toBeChecked();
		expect(
			screen.getByRole("checkbox", { name: "Auto-update" }),
		).not.toBeChecked();
		vi.mocked(invokeClient).mockClear();
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenLastCalledWith({
			...defaultDraft,
			autoUpdate: false,
		});
		expect(
			vi
				.mocked(invokeClient)
				.mock.calls.some(
					([command]) => command === "update_performance_telemetry",
				),
		).toBe(false);
		await user.click(metrics);
		await user.click(screen.getByRole("button", { name: "Save" }));
		await waitFor(() =>
			expect(invokeClient).toHaveBeenCalledWith(
				"update_performance_telemetry",
				{ enabled: true },
			),
		);
		expect(onSave).toHaveBeenLastCalledWith({
			...defaultDraft,
			autoUpdate: false,
		});
		vi.mocked(invokeClient).mockClear();
	});

	it("親の設定購読が後から届いても他欄の保存でmetricsの値を戻さない", async () => {
		publishDefaults({ desktop: false });
		const { invokeClient } = await import("@/lib/client");
		const user = userEvent.setup();
		function SettingsScreen() {
			const { settings, loaded, loadError, updateSettings } = useSettings();
			return (
				<>
					<p>Parent metrics: {String(settings.performanceTelemetry)}</p>
					<SettingsModal
						{...defaultProps}
						settings={settings}
						desktopSettingsLoaded={loaded}
						desktopSettingsError={loadError}
						onSave={updateSettings}
					/>
				</>
			);
		}
		render(<SettingsScreen />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const metrics = screen.getByRole("checkbox", {
			name: "Send anonymous performance metrics",
		});
		expect(metrics).toBeDisabled();
		await user.click(screen.getByRole("checkbox", { name: "Auto-update" }));
		await act(async () =>
			states.publish("desktop-settings", {
				...desktopSettings,
				performanceTelemetry: false,
			}),
		);
		expect(metrics).toBeEnabled();
		expect(metrics).not.toBeChecked();
		vi.mocked(invokeClient).mockClear();
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(screen.getByText("Parent metrics: false")).toBeVisible();
		expect(invokeClient).not.toHaveBeenCalledWith(
			"update_performance_telemetry",
			expect.anything(),
		);
		await user.click(metrics);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(invokeClient).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: true,
		});
		expect(screen.getByText("Parent metrics: false")).toBeVisible();
		await act(async () => states.publish("desktop-settings", desktopSettings));
		expect(screen.getByText("Parent metrics: true")).toBeVisible();
		expect(
			screen.getByRole("checkbox", { name: "Auto-update" }),
		).not.toBeChecked();
		vi.mocked(invokeClient).mockClear();
	});

	it("開き直した最初の描画からmetricsと他欄は親の値になる", async () => {
		const user = userEvent.setup();
		const commits: Array<{
			metrics: string | null;
			autoUpdate: string | null;
		}> = [];
		const onRender = () => {
			const metrics = document.getElementById("performance-telemetry");
			if (metrics)
				commits.push({
					metrics: metrics.getAttribute("data-state"),
					autoUpdate:
						document
							.getElementById("auto-update")
							?.getAttribute("data-state") ?? null,
				});
		};
		const modal = (open: boolean, settings = defaultSettings) => (
			<Profiler id="settings" onRender={onRender}>
				<SettingsModal {...defaultProps} open={open} settings={settings} />
			</Profiler>
		);
		const view = render(modal(true));
		fireEvent.click(screen.getByText("Privacy & Updates"));
		await user.click(
			screen.getByRole("checkbox", {
				name: "Send anonymous performance metrics",
			}),
		);
		await user.click(screen.getByRole("checkbox", { name: "Auto-update" }));
		view.rerender(modal(true, { ...defaultSettings, fontSize: 18 }));
		expect(
			screen.getByRole("checkbox", {
				name: "Send anonymous performance metrics",
			}),
		).not.toBeChecked();
		view.rerender(modal(false));
		commits.length = 0;
		view.rerender(modal(true));
		expect(commits.length).toBeGreaterThan(0);
		expect(commits[0]).toEqual({ metrics: "checked", autoUpdate: "checked" });
	});

	it.each(["background", "metrics"])(
		"%sの保存失敗後もmetricsの未保存値を再試行する",
		async (failure) => {
			const { invokeClient } = await import("@/lib/client");
			const user = userEvent.setup();
			function SettingsScreen() {
				const { settings, loaded, loadError, updateSettings } = useSettings();
				return (
					<>
						<p>Parent metrics: {String(settings.performanceTelemetry)}</p>
						<SettingsModal
							{...defaultProps}
							settings={settings}
							desktopSettingsLoaded={loaded}
							desktopSettingsError={loadError}
							onSave={updateSettings}
						/>
					</>
				);
			}
			render(<SettingsScreen />);
			await user.click(screen.getByText("Privacy & Updates"));
			await user.click(
				screen.getByRole("checkbox", {
					name: "Send anonymous performance metrics",
				}),
			);
			if (failure === "background") {
				await user.click(screen.getByText("Background"));
				await user.click(
					screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
				);
			}
			const command =
				failure === "background"
					? "update_app_settings"
					: "update_performance_telemetry";
			vi.mocked(invokeClient).mockClear();
			vi.mocked(invokeClient).mockImplementationOnce(() =>
				Promise.reject(new Error(`${failure} write failed`)),
			);
			await user.click(screen.getByRole("button", { name: "Save" }));
			expect(await screen.findByRole("alert")).toHaveTextContent(
				`${failure} write failed`,
			);
			expect(screen.getByText("Parent metrics: true")).toBeVisible();
			expect(invokeClient).toHaveBeenCalledWith(command, expect.anything());
			expect(invokeClient).not.toHaveBeenCalledWith("report_usage_event", {
				name: "settings_saved",
			});
			if (failure === "background")
				expect(invokeClient).not.toHaveBeenCalledWith(
					"update_performance_telemetry",
					expect.anything(),
				);
			await user.click(screen.getByText("Privacy & Updates"));
			expect(
				screen.getByRole("checkbox", {
					name: "Send anonymous performance metrics",
				}),
			).not.toBeChecked();
			expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
			vi.mocked(invokeClient).mockClear();
			await user.click(screen.getByRole("button", { name: "Save" }));
			expect(invokeClient).toHaveBeenCalledWith(
				"update_performance_telemetry",
				{ enabled: false },
			);
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(screen.getByText("Parent metrics: true")).toBeVisible();
			await act(async () =>
				states.publish("desktop-settings", {
					...desktopSettings,
					performanceTelemetry: false,
				}),
			);
			expect(screen.getByText("Parent metrics: false")).toBeVisible();
			vi.mocked(invokeClient).mockClear();
		},
	);

	it("ログイン項目の初期取得失敗後の保存を成功として記録しない", async () => {
		vi.mocked(invokeTauri).mockRejectedValueOnce(
			new Error("login unavailable"),
		);
		const { invokeClient } = await import("@/lib/client");
		const user = userEvent.setup();
		render(<SettingsModal {...defaultProps} />);
		await user.click(screen.getByText("Background"));
		expect(await screen.findByRole("alert")).toHaveTextContent(
			"login unavailable",
		);
		const closeToTray = screen.getByRole("checkbox", {
			name: "Minimize to tray on close",
		});
		await user.click(closeToTray);
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(screen.getByRole("alert")).toHaveTextContent(
			"Background settings are not loaded.",
		);
		expect(closeToTray).not.toBeChecked();
		expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
		expect(invokeClient).not.toHaveBeenCalledWith("report_usage_event", {
			name: "settings_saved",
		});
	});

	it("承認待ちのログイン項目は無効と案内を表示し承認先を開く", async () => {
		vi.mocked(invokeTauri).mockResolvedValue({
			enabled: false,
			requiresApproval: true,
			reason: null,
		});
		const user = userEvent.setup();
		render(<SettingsModal {...defaultProps} />);
		await user.click(screen.getByText("Background"));
		expect(await screen.findByRole("status")).toHaveTextContent(
			"Allow Releash in System Settings → General → Login Items.",
		);
		expect(
			screen.getByRole("checkbox", { name: "Launch at login" }),
		).not.toBeChecked();
		await user.click(screen.getByRole("button", { name: "Open Login Items" }));
		expect(invokeTauri).toHaveBeenCalledWith("open_login_item_settings");
	});

	it("ログイン項目を登録できない理由をalertで表示する", async () => {
		const reason =
			"Move Releash out of the read-only disk image before registering login items.";
		vi.mocked(invokeTauri).mockResolvedValue({
			enabled: false,
			requiresApproval: false,
			reason,
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Background"));
		expect(await screen.findByRole("alert")).toHaveTextContent(reason);
		expect(
			screen.queryByRole("button", { name: "Open Login Items" }),
		).not.toBeInTheDocument();
	});

	it("明示したCLI設置ボタンの操作後に設置結果を表示する", async () => {
		const { invokeClient } = await import("@/lib/client");
		const message = "Releash CLI installed at /usr/local/bin/releash";
		vi.mocked(invokeClient).mockResolvedValue({
			status: "installed",
			path: "/usr/local/bin/releash",
		} as never);
		const user = userEvent.setup();
		render(<SettingsModal {...defaultProps} />);
		await user.click(screen.getByText("Background"));
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
		expect(invokeClient).not.toHaveBeenCalledWith("install_cli");
		await user.click(
			screen.getByRole("button", { name: "Install CLI command" }),
		);
		expect(invokeClient).toHaveBeenCalledWith("install_cli");
		expect(await screen.findByRole("status")).toHaveTextContent(message);
	});
	it("CLI の配置拒否を設定画面の alert に表示する", async () => {
		const { invokeClient } = await import("@/lib/client");
		const reason =
			"Move Releash.app to Applications before installing the CLI.";
		vi.mocked(invokeClient).mockRejectedValueOnce(reason);
		const user = userEvent.setup();
		render(<SettingsModal {...defaultProps} />);
		await user.click(screen.getByText("Background"));
		await user.click(
			screen.getByRole("button", { name: "Install CLI command" }),
		);
		expect(await screen.findByRole("alert")).toHaveTextContent(reason);
		expect(screen.queryByRole("status")).not.toBeInTheDocument();
	});

	it("購読の失敗を各設定に表示し回復した値で置き換える", async () => {
		const notion: StateTarget<"notion-config"> = {
			kind: "notion-config",
			args: [REPO],
		};
		const targets: StateTarget<keyof StateValues>[] = [
			"workflow-config",
			"external-editor",
			"provider-availability",
			"desktop-settings",
			notion,
		];
		states.clear();
		states.publish({ kind: "branches", args: [REPO] }, []);
		states.publish({ kind: "releash-base", args: [REPO] }, null);
		const view = render(<SettingsModal {...defaultProps} />);
		await act(async () => {
			for (const target of targets)
				states.fail(
					target,
					new ConnectError("Request failed", Code.Unavailable),
				);
		});
		for (const section of ["Editor", "Notion", "Agent", "Background"]) {
			fireEvent.click(screen.getByText(section));
			await waitFor(() =>
				expect(
					screen.getAllByText(/処理中にエラーが発生しました/).length,
				).toBeGreaterThan(0),
			);
			if (section === "Notion") {
				expect(screen.getByRole("alert")).toHaveTextContent("/repos/my-app");
				expect(screen.queryByLabelText("API Token")).not.toBeInTheDocument();
			}
		}
		expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
		await act(async () => {
			states.publish(notion, {
				api_token: "recovered-token",
				database_id: "recovered-db",
				property_mapping: {
					title: "Title",
					labels: [],
					branch_name: "Branch",
					branch_prefix: "fix/",
				},
			});
			states.publish("provider-availability", {
				providers: [
					{
						provider: "codex",
						displayName: "Codex",
						defaultExecutable: "codex",
						configuredExecutable: "/recovered/codex",
						configurationRevision: 0,
						effectiveExecutable: "/recovered/codex",
						available: true,
						resolvedExecutable: "/recovered/codex",
						unavailableReason: null,
					},
				],
			});
			states.publish("desktop-settings", {
				...desktopSettings,
				closeToTray: false,
				startMinimized: true,
				autoLaunch: true,
			});
			states.publish("external-editor", {
				selected: "/bin/zed",
				editors: [{ name: "Zed", path: "/bin/zed" }],
			});
			states.publish("workflow-config", { approval_auto_approve: true });
		});
		await waitFor(() =>
			expect(
				screen.queryByText(/処理中にエラーが発生しました/),
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
			screen.queryByText(/処理中にエラーが発生しました/),
		).not.toBeInTheDocument();
		expect(screen.getByLabelText("Codex executable override")).toHaveValue(
			"/recovered/codex",
		);
		expect(screen.queryByText("not_found")).not.toBeInTheDocument();
		fireEvent.click(screen.getByText("Editor"));
		expect(
			screen.queryByText(/処理中にエラーが発生しました/),
		).not.toBeInTheDocument();
		expect(
			screen.getByRole("combobox", { name: "External Editor" }),
		).toHaveTextContent("Zed");
		view.unmount();
	});

	it.each(["editor", "workflow", "base", "background", "notion", "provider"])(
		"再取得しても%sの未保存入力と保存可能な状態を保持する",
		async (form) => {
			vi.mocked(invokeTauri).mockResolvedValue({
				enabled: true,
				requiresApproval: false,
				reason: null,
			});
			const republish = () => {
				states.publish("external-editor", { selected: "code", editors });
				states.publish("desktop-settings", desktopSettings);
				states.publish({ kind: "releash-base", args: [REPO] }, "main");
				states.publish("workflow-config", { approval_auto_approve: false });
				states.publish("provider-availability", providerSnapshot);
				states.publish({ kind: "notion-config", args: [REPO] }, null);
			};
			republish();
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
			await act(async () => republish());
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
		const { invokeClient: invoke } = await import("@/lib/client");
		states.publish("provider-availability", {
			providers: [
				{
					provider: "claude",
					displayName: "Claude",
					defaultExecutable: "claude",
					configuredExecutable: null,
					configurationRevision: 0,
					effectiveExecutable: "claude",
					available: true,
					resolvedExecutable: "/usr/bin/claude",
					unavailableReason: null,
				},
			],
		});
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "update_provider_executable") {
				return Promise.resolve(null as never);
			}
			return Promise.resolve(null as never);
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
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockResolvedValue(null as never);
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		await user.click(
			await screen.findByRole("button", { name: "Reset Claude executable" }),
		);
		expect(screen.getByLabelText("Claude executable override")).toHaveValue(
			"/opt/custom/claude",
		);
		expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
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
		const { invokeClient: invoke } = await import("@/lib/client");
		const provider = (id: string, configuredExecutable: string | null) => ({
			provider: id,
			displayName: id === "claude" ? "Claude" : "Codex",
			defaultExecutable: id,
			configuredExecutable,
			configurationRevision: 0,
			effectiveExecutable: configuredExecutable ?? id,
			available: true,
			resolvedExecutable: `/usr/bin/${id}`,
			unavailableReason: null,
		});
		states.publish("provider-availability", {
			providers: [
				provider("claude", "/opt/custom/claude"),
				provider("codex", null),
			],
		});
		vi.mocked(invoke).mockImplementation((command: string) => {
			if (command === "reset_provider_executable") {
				return Promise.resolve(null as never);
			}
			return Promise.resolve(null as never);
		});
		render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		const codex = await screen.findByLabelText("Codex executable override");
		await user.type(codex, "/draft/codex");

		await user.click(
			screen.getByRole("button", { name: "Reset Claude executable" }),
		);
		act(() =>
			states.publish("provider-availability", {
				providers: [provider("claude", null), provider("codex", null)],
			}),
		);

		expect(codex).toHaveValue("/draft/codex");
	});

	it("Provider CLI refresh失敗時は直前snapshotを維持してerrorを表示する", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/client");
		states.publish("provider-availability", {
			providers: [
				{
					provider: "dynamic-provider",
					displayName: "Dynamic Provider",
					defaultExecutable: "dynamic",
					configuredExecutable: null,
					configurationRevision: 0,
					effectiveExecutable: "dynamic",
					available: true,
					resolvedExecutable: "/bin/dynamic",
					unavailableReason: null,
				},
			],
		});
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			if (cmd === "refresh_provider_availability") {
				return Promise.reject(new Error("refresh failed"));
			}
			return Promise.resolve(null as never);
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
			...defaultDraft,
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
			...defaultDraft,
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
			...defaultDraft,
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
		const { invokeClient: invoke } = await import("@/lib/client");
		render(<SettingsModal {...defaultProps} onSave={onSave} />);
		fireEvent.click(screen.getByText("Privacy & Updates"));
		const checkbox = screen.getByRole("checkbox", {
			name: "Send anonymous performance metrics",
		});
		await user.click(checkbox);
		await user.click(screen.getByRole("button", { name: "Save" }));
		expect(onSave).toHaveBeenCalledWith(defaultDraft);
		expect(invoke).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: false,
		});
	});

	it("should re-enable performance telemetry and call onSave", async () => {
		states.publish("desktop-settings", {
			...desktopSettings,
			performanceTelemetry: false,
		});
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invokeClient: invoke } = await import("@/lib/client");
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
		expect(onSave).toHaveBeenCalledWith(defaultDraft);
		expect(invoke).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: true,
		});
	});

	it("should call settings_saved after performance telemetry update completes", async () => {
		const user = userEvent.setup();
		const onSave = vi.fn();
		const { invokeClient: invoke } = await import("@/lib/client");
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
						resolve(null as never);
					};
				});
			}
			if (cmd === "report_usage_event") {
				callOrder.push("settings_saved");
				return Promise.resolve(null as never);
			}
			return Promise.resolve(null as never);
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
		render(<SettingsModal {...defaultProps} />);
		expect(screen.getByText("Repositories")).toBeInTheDocument();
		fireEvent.click(screen.getByText("Repositories"));
		expect(await screen.findByText("Base branch")).toBeInTheDocument();
	});

	it("should load and save approval auto-approve independently from agent auto-approve", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/client");
		states.publish("workflow-config", { approval_auto_approve: true });
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "update_workflow_config":
					return Promise.resolve(null as never);
				default:
					return Promise.resolve(null as never);
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

		expect(invoke).toHaveBeenCalledWith("update_workflow_config", {
			workflow: { approval_auto_approve: false },
		});
	});

	it("should save external editor selection via Save button", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/client");
		states.publish("external-editor", {
			selected: "",
			editors: [
				{
					name: "Visual Studio Code",
					path: "/Applications/Visual Studio Code.app",
				},
				{ name: "Cursor", path: "/Applications/Cursor.app" },
			],
		});
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "update_external_editor":
					return Promise.resolve(null as never);
				default:
					return Promise.resolve(null as never);
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
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "set_releash_base":
					return Promise.resolve(null as never);
				default:
					return Promise.resolve(null as never);
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
		subscribeStates({
			workflows: [
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
			],
			diagnostics: EMPTY_REPORT,
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
		const { invokeClient: invoke } = await import("@/lib/client");
		subscribeStates({
			workflows: [
				{
					name: "my-workflow",
					description: "カスタムワークフロー",
					builtin: false,
					sourceFormat: "yaml" as const,
					is_running: false,
				},
			],
			"workflow-source:my-workflow": "name: my-workflow\nnodes: []\n",
			"workflow:my-workflow": {
				name: "my-workflow",
				description: "カスタムワークフロー",
				builtin: false,
				sourceFormat: "yaml",
				nodes: [],
			},
			diagnostics: EMPTY_REPORT,
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
		expect(vi.mocked(subscribeState)).toHaveBeenCalledWith(
			{ kind: "workflow-source", args: ["my-workflow"] },
			expect.anything(),
			expect.anything(),
		);
		expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
			"open_workflow_in_editor",
			expect.anything(),
		);
	});

	it("should call delete_workflow when Delete button is clicked", async () => {
		const user = userEvent.setup();
		const { invokeClient: invoke } = await import("@/lib/client");
		subscribeStates({
			workflows: [
				{
					name: "my-workflow",
					description: "カスタムワークフロー",
					builtin: false,
					sourceFormat: "yaml" as const,
					is_running: false,
				},
			],
			diagnostics: EMPTY_REPORT,
		});
		vi.mocked(invoke).mockResolvedValue(null as never);

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
		subscribeStates({
			workflows: [
				{
					name: "quick-fix",
					description: "素早いバグ修正",
					builtin: true,
					sourceFormat: "yaml" as const,
					is_running: false,
				},
			],
			diagnostics: EMPTY_REPORT,
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
			const { invokeClient: invoke } = await import("@/lib/client");
			vi.mocked(invoke).mockResolvedValue(null as never);
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
		states.publish("external-editor", { selected: "code", editors });
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
			states.publish("external-editor", { selected: "code", editors });
			states.publish("workflow-config", { approval_auto_approve: false });
			states.publish("provider-availability", providerSnapshot);
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
	it.each(["success", "failure", "timeout"])(
		"workflow保存は通信状態を表示せず応答の%sを反映する",
		async (outcome) => {
			const client = await import("@/lib/client");
			const { invokeClient: invoke } = client;
			const base = vi.mocked(invoke).getMockImplementation();
			if (!base) throw new Error("Missing invoke fixture");
			vi.mocked(invoke).mockClear();
			let options: unknown;
			let complete!: () => void;
			let fail!: (error: Error) => void;
			vi.mocked(invoke).mockImplementation(
				(command, args, ...extra: unknown[]) => {
					if (command !== "update_workflow_config") return base(command, args);
					options = extra[0];
					return new Promise<void>((resolve, reject) => {
						complete = resolve;
						fail = reject;
					});
				},
			);
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Agent"));
			const checkbox = await screen.findByRole("checkbox", {
				name: "Approval auto-approve",
			});
			await userEvent.click(checkbox);
			const save = screen.getByRole("button", { name: "Save" });
			await userEvent.click(save);
			expect(options).toBeUndefined();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(save).toBeDisabled();
			expect(
				screen.queryByRole("button", { name: "元の操作の結果を確認" }),
			).not.toBeInTheDocument();
			expect(
				vi
					.mocked(invoke)
					.mock.calls.filter(
						([command]) => command === "update_workflow_config",
					),
			).toHaveLength(1);
			await act(async () => {
				if (outcome === "success") complete();
				else
					fail(
						outcome === "timeout"
							? new ConnectError("deadline exceeded", Code.DeadlineExceeded)
							: new Error("保存が拒否されました"),
					);
			});
			if (outcome === "success") {
				expect(screen.queryByRole("alert")).not.toBeInTheDocument();
				expect(save).toBeDisabled();
				expect(checkbox).toBeChecked();
			} else {
				expect(screen.getByRole("alert")).toHaveTextContent(
					outcome === "timeout"
						? "処理中にエラーが発生しました"
						: "保存が拒否されました",
				);
				expect(save).toBeEnabled();
			}
		},
	);

	it.each(["success", "failure"])(
		"背景設定保存は通信状態を表示せず応答の%sを反映する",
		async (outcome) => {
			const client = await import("@/lib/client");
			const { invokeClient: invoke } = client;
			const base = vi.mocked(invoke).getMockImplementation();
			if (!base) throw new Error("Missing invoke fixture");
			vi.mocked(invoke).mockClear();
			let options: unknown;
			let complete!: () => void;
			let fail!: (error: Error) => void;
			vi.mocked(invoke).mockImplementation(
				(command, args, ...extra: unknown[]) => {
					if (command !== "update_app_settings") return base(command, args);
					options = extra[0];
					return new Promise<void>((resolve, reject) => {
						complete = resolve;
						fail = reject;
					});
				},
			);
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Background"));
			const checkbox = await screen.findByRole("checkbox", {
				name: "Minimize to tray on close",
			});
			await userEvent.click(checkbox);
			const save = screen.getByRole("button", { name: "Save" });
			await userEvent.click(save);
			expect(save.querySelector(".animate-spin")).not.toBeNull();
			expect(options).toBeUndefined();
			expect(save).toBeDisabled();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			fireEvent.click(screen.getByText("Appearance"));
			expect(
				screen.queryByRole("button", { name: "元の操作の結果を確認" }),
			).not.toBeInTheDocument();
			fireEvent.click(screen.getByText("Background"));
			await act(async () =>
				states.publish("desktop-settings", desktopSettings),
			);
			expect(
				screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
			).not.toBeChecked();
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
		"%sは通信状態を表示せず購読結果を反映する: failure=%s",
		async (command, failure) => {
			const client = await import("@/lib/client");
			const invoke = vi.mocked(client.invokeClient);
			const base = invoke.getMockImplementation();
			invoke.mockClear();
			if (!base) throw new Error("Missing settings fixture");
			let options: unknown;
			let complete!: (
				value: Awaited<ReturnType<typeof client.invokeClient>>,
			) => void;
			let fail!: (error: Error) => void;
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
			invoke.mockImplementation((name, args, ...extra: unknown[]) => {
				if (name === command) {
					options = extra[0];
					return new Promise((resolve, reject) => {
						complete = resolve;
						fail = reject;
					});
				}
				return base(name, args);
			});
			const notion: StateTarget<"notion-config"> = {
				kind: "notion-config",
				args: [REPO],
			};
			const releashBase: StateTarget<"releash-base"> = {
				kind: "releash-base",
				args: [REPO],
			};
			states.publish("external-editor", { selected: "code", editors });
			states.publish(releashBase, "main");
			states.publish(notion, config);
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
			expect(options).toBeUndefined();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "元の操作の結果を確認" }),
			).not.toBeInTheDocument();
			await userEvent.click(button);
			expect(
				invoke.mock.calls.filter(([name]) => name === command),
			).toHaveLength(1);
			await act(async () => {
				if (failure) fail(new Error("変更が拒否されました"));
				else {
					complete(null as never);
					states.publish("external-editor", { selected: "zed", editors });
					states.publish(releashBase, "develop");
					states.publish(
						notion,
						command === "delete_notion_config"
							? null
							: { ...config, api_token: "saved" },
					);
					states.publish("provider-availability", nextProviders);
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
		"%sの応答待ちで通信状態や結果照会を表示しない",
		async (command) => {
			const client = await import("@/lib/client");
			const invoke = vi.mocked(client.invokeClient);
			const base = invoke.getMockImplementation();
			if (!base) throw new Error("Missing settings fixture");
			let complete!: () => void;
			invoke.mockImplementation((name, args) => {
				if (name !== command) return base(name, args);
				return new Promise((resolve) => {
					complete = () => resolve(null as never);
				});
			});
			render(<SettingsModal {...defaultProps} />);
			fireEvent.click(screen.getByText("Agent"));
			const input = await screen.findByLabelText("Claude executable override");
			if (command === "update_provider_executable")
				fireEvent.change(input, { target: { value: "/saved/claude" } });
			const button = screen.getByRole("button", {
				name:
					command === "update_provider_executable"
						? "Save"
						: command === "reset_provider_executable"
							? "Reset Claude executable"
							: "Refresh Provider CLI availability",
			});
			await userEvent.click(button);
			expect(button).toBeDisabled();
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "元の操作の結果を確認" }),
			).not.toBeInTheDocument();
			await act(async () => complete());
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
		},
	);

	it("providerフォームを開き直すと取得中と取得失敗時にも以前のdraftを保存しない", async () => {
		const { invokeClient: invoke } = await import("@/lib/client");
		vi.mocked(invoke).mockClear();
		const view = render(<SettingsModal {...defaultProps} />);
		fireEvent.click(screen.getByText("Agent"));
		fireEvent.change(await screen.findByDisplayValue("/opt/custom/claude"), {
			target: { value: "/unsaved/claude" },
		});
		expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
		view.rerender(<SettingsModal {...defaultProps} open={false} />);
		publishDefaults({ provider: false });
		view.rerender(<SettingsModal {...defaultProps} open />);
		fireEvent.click(screen.getByText("Agent"));
		expect(
			screen.queryByDisplayValue("/unsaved/claude"),
		).not.toBeInTheDocument();
		expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
		await act(async () =>
			states.fail("provider-availability", new Error("取得できません")),
		);
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
	for (const kind of ["branches", "releash-base"] as const) {
		for (const received of [false, true]) {
			it(`${kind}の失敗を${received ? "前の値と一緒に" : "未設定と区別して"}表示する`, async () => {
				const user = userEvent.setup();
				const target = { kind, args: [REPO] };
				states.clear();
				// Other read succeeds so this target's failure is observable.
				states.publish({ kind: "branches", args: [REPO] }, [
					{ name: "develop", is_remote: false },
				]);
				if (kind === "branches" || received)
					states.publish({ kind: "releash-base", args: [REPO] }, "develop");
				if (kind === "branches" && !received) {
					states.clear();
					states.publish({ kind: "releash-base", args: [REPO] }, "develop");
				}
				render(<SettingsModal {...defaultProps} />);
				await user.click(screen.getByText("Repositories"));
				act(() => states.fail(target, new Error(`${kind} unavailable`)));
				expect(await screen.findByRole("alert")).toHaveTextContent(
					`${kind} unavailable`,
				);
				expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
				if (received) {
					const trigger = screen.getByRole("combobox", { name: "Base branch" });
					expect(trigger).toHaveTextContent("develop");
					if (kind === "branches") {
						await user.click(trigger);
						expect(
							screen.getByRole("option", { name: "develop" }),
						).toBeInTheDocument();
					}
				}
			});
		}
	}
});
