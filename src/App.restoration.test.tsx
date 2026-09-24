import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { WorkspaceListModel } from "@/hooks/useWorkspaceList";
import { completeClientRestoration, invokeClient } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { workspaceListSnapshot } from "@/test/workspaceList";
import type { AppSettings } from "@/types/settings";
import App from "./App";

const states = stateSubscriptions();
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	listenClient: vi.fn().mockResolvedValue(() => {}),
	watchClient: vi.fn().mockReturnValue(() => {}),
	completeClientRestoration: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
	firstState: (...args: Parameters<typeof states.firstState>) =>
		states.firstState(...args),
}));
vi.mock("@/hooks/useMenuEvents", () => ({ useMenuEvents: vi.fn() }));
vi.mock("@/hooks/useUpdateChecker", () => ({ useUpdateChecker: () => null }));
vi.mock("@/hooks/useWorkspaceNavigation", () => ({
	useWorkspaceNavigation: () => ({
		worktrees: [],
		selectedWorktreeId: null,
		openWorktreeTab: vi.fn(),
	}),
}));
vi.mock("@/components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("@/components/panels/SettingsModal", () => ({
	SettingsModal: ({
		open,
		repoPaths,
	}: {
		open: boolean;
		repoPaths: string[];
	}) =>
		open ? (
			<section aria-label="Registered repositories">
				{repoPaths.join(",")}
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
	connectionGeneration: number;
	retryAvailable: boolean;
	stage: string | null;
	reason: string | null;
};
beforeEach(() => {
	vi.useFakeTimers();
	vi.clearAllMocks();
	localStorage.clear();
	states.clear();
	states.publish("repository-paths", []);
	states.publish("workspaces", workspaceListSnapshot());
	status = {
		phase: "restoring",
		connectionGeneration: 1,
		retryAvailable: false,
		stage: null,
		reason: null,
	};
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "get_application_startup_outcome") return { type: "ready" };
		if (command === "get_daemon_status") return { ...status };
		if (command === "fail_desktop_restoration") {
			status = {
				...status,
				phase: "failed",
				stage: "state_restoration",
				reason: (args as { reason: string }).reason,
				retryAvailable: true,
			};
		}
		if (command === "retry_daemon") {
			status = {
				...status,
				phase: "restoring",
				connectionGeneration: status.connectionGeneration + 1,
				stage: null,
				reason: null,
				retryAvailable: false,
			};
		}
	});
	vi.mocked(completeClientRestoration).mockImplementation(async () => {
		status = { ...status, phase: "ready" };
	});
});
afterEach(() => vi.useRealTimers());

it("設定の初回失敗後は次の接続の状態を反映してから操作を再開する", async () => {
	states.clear();
	let resolveSettings!: (value: boolean) => void;
	const settings = new Promise<boolean>((resolve) => {
		resolveSettings = resolve;
	});
	vi.mocked(invokeClient).mockImplementation((command) => {
		if (command === "get_performance_telemetry_enabled")
			return status.connectionGeneration === 1
				? Promise.reject(new Error("temporary read failure"))
				: settings;
		return Promise.resolve(undefined);
	});
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByRole("status")).toHaveTextContent(
		"temporary read failure",
	);
	expect(completeClientRestoration).not.toHaveBeenCalled();
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Retry" }));
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	await act(async () => resolveSettings(false));
	expect(completeClientRestoration).not.toHaveBeenCalled();
	await act(async () => states.publish("workspaces", workspaceListSnapshot()));
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(completeClientRestoration).toHaveBeenCalledExactlyOnceWith(2);
	expect(screen.getByRole("main")).toHaveTextContent("Telemetry: false");
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.some(([name]) => name === "refresh_workspaces"),
	).toBe(false);
});
it("初回の一覧失敗でも復元を完了し購読による復旧を表示する", async () => {
	states.publish("workspaces", {
		generation: 1,
		repositories: [],
		status: { loaded: false, state: "initialFailed", error: "offline" },
	});
	vi.mocked(invokeClient).mockResolvedValue(true);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(completeClientRestoration).toHaveBeenCalledExactlyOnceWith(1);
	expect(screen.getByRole("main")).toBeVisible();
	await act(async () => states.publish("workspaces", workspaceListSnapshot()));
	expect(screen.getByRole("button", { name: "/repo" })).toBeVisible();
});
it("復元完了通知が失敗した場合も理由を表示し再開できる", async () => {
	vi.mocked(invokeClient).mockResolvedValue(true);
	vi.mocked(completeClientRestoration).mockRejectedValueOnce(
		new Error("restoration acknowledgement failed"),
	);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByRole("status")).toHaveTextContent(
		"restoration acknowledgement failed",
	);
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Retry" }));
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByRole("main")).toBeVisible();
	expect(completeClientRestoration).toHaveBeenLastCalledWith(2);
});
it("登録一覧とWorkspacesの変更・削除はそれぞれの購読から届く", async () => {
	vi.mocked(invokeClient).mockResolvedValue(true);
	states.publish("repository-paths", ["/repo"]);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
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
	expect(settings).toBeEmptyDOMElement();
	expect(screen.queryByRole("button", { name: "/new" })).toBeNull();
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.some(([name]) => name === "refresh_workspaces"),
	).toBe(false);
});
