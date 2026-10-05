import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { type ReactNode, StrictMode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import type { SettingsModalProps } from "@/components/panels/SettingsModal";
import type { WorkspaceListModel } from "@/hooks/useWorkspaceList";
import { invokeClient } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { workspaceListSnapshot } from "@/test/workspaceList";
import type { AppSettings } from "@/types/settings";
import App from "./App";

const states = stateSubscriptions();
const desktopSettings = {
	closeToTray: true,
	startMinimized: false,
	crashReporting: true,
	performanceTelemetry: false,
	autoLaunch: false,
};
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
	firstState: (...args: Parameters<typeof states.firstState>) =>
		states.firstState(...args),
}));
vi.mock("@/hooks/useMenuEvents", () => ({ useMenuEvents: vi.fn() }));
const openWorktreeTab = vi.fn();
let selectedWorktreeId: string | null = null;
vi.mock("@/hooks/useWorkspaceNavigation", () => ({
	useWorkspaceNavigation: () => ({
		worktrees: [],
		selectedWorktreeId,
		openWorktreeTab,
	}),
}));
vi.mock("@/components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: ({
		open,
		repoPaths,
		settings,
		onSave,
		desktopSettingsLoaded,
		desktopSettingsError,
	}: SettingsModalProps) =>
		open ? (
			<section aria-label="Registered repositories">
				<span data-testid="registered-repositories">
					{repoPaths?.join(",")}
				</span>
				<p>Settings loaded: {String(desktopSettingsLoaded)}</p>
				{desktopSettingsError && <p role="alert">{desktopSettingsError}</p>}
				<button
					type="button"
					onClick={() =>
						onSave({ ...settings, autoUpdate: !settings.autoUpdate })
					}
				>
					Toggle auto-update
				</button>
			</section>
		) : null,
}));
vi.mock("@/screens/MainLayout", () => ({
	MainLayout: ({
		settings,
		leftNav,
	}: {
		settings: AppSettings;
		leftNav: ReactNode;
	}) => (
		<main>
			<p>Telemetry: {String(settings.performanceTelemetry)}</p>
			{leftNav}
		</main>
	),
}));
vi.mock("@/components/workspace/WorkspaceList", () => ({
	WorkspaceList: ({
		model,
		onShowSettings,
	}: {
		model: WorkspaceListModel;
		onShowSettings: () => void;
	}) => {
		return (
			<div>
				<button type="button" onClick={onShowSettings}>
					Settings
				</button>
				<button type="button" onClick={() => void model.refresh()}>
					Refresh Workspaces
				</button>
				{model.snapshot?.repositories.map(({ path }) => (
					<button type="button" key={path}>
						{path}
					</button>
				))}
			</div>
		);
	},
}));

let status: {
	phase: string;
	retryAvailable: boolean;
	stage: string | null;
	reason: string | null;
};
let channel: { onmessage?: (next: typeof status) => void } | null;
const notifyStatus = () => channel?.onmessage?.({ ...status });
beforeEach(() => {
	vi.clearAllMocks();
	localStorage.clear();
	states.clear();
	states.publish("repository-paths", []);
	states.publish("workspaces", workspaceListSnapshot());
	states.publish("desktop-settings", desktopSettings);
	status = {
		phase: "ready",
		retryAvailable: false,
		stage: null,
		reason: null,
	};
	channel = null;
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "subscribe_daemon_status") {
			channel = (args as { channel: typeof channel }).channel;
			notifyStatus();
			return;
		}
	});
});

it("設定と一覧の初回失敗でシェルをFailedにせず購読の復旧を表示する", async () => {
	states.clear();
	vi.mocked(invokeClient).mockResolvedValue(true);
	await act(async () => {
		render(<App />);
	});
	const main = screen.getByRole("main");
	await act(async () =>
		states.fail("desktop-settings", new Error("temporary read failure")),
	);
	expect(screen.getByRole("main")).toBe(main);
	expect(screen.queryByRole("region", { name: "Daemon status" })).toBeNull();
	await act(async () => {
		states.publish("desktop-settings", desktopSettings);
		states.publish("workspaces", workspaceListSnapshot());
	});
	expect(main).toHaveTextContent("Telemetry: false");
	expect(screen.getByRole("button", { name: "/repo" })).toBeVisible();
	expect(
		vi
			.mocked(invoke)
			.mock.calls.some(([command]) => command.includes("restoration")),
	).toBe(false);
});

it("再接続後も起動処理と更新確認は一度だけでReady復帰時にメニューを同期する", async () => {
	selectedWorktreeId = null;
	vi.mocked(invokeClient).mockResolvedValue(true);
	states.publish("startup-repository", {
		path: "/repo",
		branch: "main",
		repositoryName: "repo",
	});
	states.publish({ kind: "worktrees", args: ["/repo"] }, [
		{
			path: "/repo",
			branch: "main",
			name: "repo",
			is_main: true,
			is_locked: false,
		},
	]);
	await act(async () => {
		render(<App />);
	});
	expect(openWorktreeTab).toHaveBeenCalledTimes(1);
	expect(invokeClient).not.toHaveBeenCalledWith(
		"add_repo_path",
		expect.anything(),
	);
	const main = screen.getByRole("main");
	fireEvent.click(screen.getByRole("button", { name: "Settings" }));
	for (const phase of ["starting", "backoff", "ready", "starting", "ready"]) {
		status = { ...status, phase };
		if (phase === "starting") selectedWorktreeId = "selected";
		await act(async () => notifyStatus());
		expect(screen.getByRole("main")).toBe(main);
		expect(
			screen.getByRole("region", { name: "Registered repositories" }),
		).toBeVisible();
	}
	expect(openWorktreeTab).toHaveBeenCalledTimes(1);
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.filter(([command]) => command === "add_repo_path"),
	).toHaveLength(0);
	expect(
		vi
			.mocked(invoke)
			.mock.calls.filter(([command]) => command === "check_desktop_update"),
	).toHaveLength(1);
	expect(
		vi
			.mocked(invoke)
			.mock.calls.filter(([command]) => command === "set_menu_items_enabled")
			.map(([, args]) => args),
	).toEqual([{ enabled: false }, { enabled: true }, { enabled: true }]);
	selectedWorktreeId = null;
});
it("登録一覧とWorkspacesの変更・削除はそれぞれの購読から届く", async () => {
	vi.mocked(invokeClient).mockResolvedValue(true);
	states.publish("repository-paths", ["/repo"]);
	await act(async () => {
		render(<App />);
	});
	fireEvent.click(screen.getByRole("button", { name: "Settings" }));
	const settings = screen.getByRole("region", {
		name: "Registered repositories",
	});
	expect(settings).toHaveTextContent("/repo");
	expect(screen.getByRole("button", { name: "/repo" })).toBeVisible();
	const next = workspaceListSnapshot();
	next.repositories[0].path = "/new";
	await act(async () => {
		states.publish("repository-paths", ["/new"]);
		states.publish("workspaces", next);
	});
	expect(settings).toHaveTextContent("/new");
	expect(screen.getByRole("button", { name: "/new" })).toBeVisible();
	expect(screen.queryByRole("button", { name: "/repo" })).toBeNull();
	await act(async () => {
		states.publish("repository-paths", []);
		states.publish("workspaces", {
			...next,
			repositories: [],
			status: { loaded: true, state: "empty", error: null },
		});
	});
	expect(screen.getByTestId("registered-repositories")).toBeEmptyDOMElement();
	expect(screen.queryByRole("button", { name: "/new" })).toBeNull();
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.some(([name]) => name === "refresh_workspaces"),
	).toBe(false);
});

it("StrictModeで初回表示と再接続を経ても自動更新確認は一度だけ", async () => {
	vi.mocked(invokeClient).mockResolvedValue(true);
	await act(async () => {
		render(
			<StrictMode>
				<App />
			</StrictMode>,
		);
	});
	for (const phase of ["starting", "ready", "backoff", "ready"]) {
		status = { ...status, phase };
		await act(async () => notifyStatus());
	}
	expect(
		vi
			.mocked(invoke)
			.mock.calls.filter(([command]) => command === "check_desktop_update"),
	).toHaveLength(1);
});

it("設定画面で自動更新をONにすると確認し成功後の切替と再接続では再確認しない", async () => {
	localStorage.setItem(
		"releash-settings",
		JSON.stringify({ autoUpdate: false }),
	);
	await act(async () => {
		render(<App />);
	});
	fireEvent.click(screen.getByRole("button", { name: "Settings" }));
	const checks = () =>
		vi
			.mocked(invoke)
			.mock.calls.filter(([command]) => command === "check_desktop_update");
	expect(checks()).toHaveLength(0);
	await act(async () =>
		fireEvent.click(screen.getByRole("button", { name: "Toggle auto-update" })),
	);
	expect(checks()).toHaveLength(1);
	for (let i = 0; i < 2; i++) {
		await act(async () =>
			fireEvent.click(
				screen.getByRole("button", { name: "Toggle auto-update" }),
			),
		);
	}
	for (const phase of ["starting", "ready"]) {
		status = { ...status, phase };
		await act(async () => notifyStatus());
	}
	expect(checks()).toHaveLength(1);
});

it("設定の読み込み状態と失敗を設定画面へ渡し回復を反映する", async () => {
	states.clear();
	states.publish("startup-repository", null);
	await act(async () => {
		render(<App />);
	});
	fireEvent.click(screen.getByRole("button", { name: "Settings" }));
	expect(screen.getByText("Settings loaded: false")).toBeVisible();
	await act(async () =>
		states.fail("desktop-settings", new Error("settings unavailable")),
	);
	expect(screen.getByRole("alert")).toHaveTextContent("settings unavailable");
	await act(async () => states.publish("desktop-settings", desktopSettings));
	expect(screen.getByText("Settings loaded: true")).toBeVisible();
	expect(screen.queryByRole("alert")).toBeNull();
});

it("メニューの有効切替の拒否を原因とともに画面へ通知する", async () => {
	const base = vi.mocked(invoke).getMockImplementation();
	const notice = vi.fn();
	window.addEventListener("releash-client-error", notice);
	try {
		vi.mocked(invoke).mockImplementation((command, args) =>
			command === "set_menu_items_enabled"
				? Promise.reject(new Error("menu unavailable"))
				: (base?.(command, args) ?? Promise.resolve()),
		);
		await act(async () => {
			render(<App />);
		});
		expect(await screen.findByRole("alert")).toHaveTextContent(
			"menu unavailable",
		);
		expect(notice).toHaveBeenCalledWith(
			expect.objectContaining({ detail: "menu unavailable" }),
		);
	} finally {
		window.removeEventListener("releash-client-error", notice);
	}
});
