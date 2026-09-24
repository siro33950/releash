import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { WorkspaceListModel } from "@/hooks/useWorkspaceList";
import { completeClientRestoration, invokeClient } from "@/lib/client";
import { workspaceListSnapshot } from "@/test/workspaceList";
import type { AppSettings } from "@/types/settings";
import App from "./App";

vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	listenClient: vi.fn().mockResolvedValue(() => {}),
	watchClient: vi.fn().mockReturnValue(() => {}),
	completeClientRestoration: vi.fn(),
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
	status = {
		phase: "restoring",
		connectionGeneration: 1,
		retryAvailable: false,
		stage: null,
		reason: null,
	};
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "subscribe_client_state") {
			(
				args as { channel: { onmessage: (paths: string[]) => void } }
			).channel.onmessage([]);
		}
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

it.each(["get_performance_telemetry_enabled"] as const)(
	"%s の初回失敗後に再取得した状態を反映してから操作を再開する",
	async (failedCommand) => {
		let resolveRepos!: (paths: string[]) => void;
		let resolveSettings!: (enabled: boolean) => void;
		const repos = new Promise<string[]>((resolve) => {
			resolveRepos = resolve;
		});
		const settings = new Promise<boolean>((resolve) => {
			resolveSettings = resolve;
		});
		vi.mocked(invokeClient).mockImplementation((command) => {
			if (command === "refresh_workspaces") {
				return repos.then((paths) => ({
					...workspaceListSnapshot(),
					repositories: paths.map((path) => ({
						...workspaceListSnapshot().repositories[0],
						path,
					})),
				}));
			}
			if (command === "get_performance_telemetry_enabled") {
				if (status.connectionGeneration === 1) {
					if (command === failedCommand)
						return Promise.reject(new Error("temporary read failure"));
					return Promise.resolve(true);
				}
				return settings;
			}
			return Promise.reject(new Error("not in a git repo"));
		});
		await act(async () => {
			render(<App />);
		});
		await act(() => vi.advanceTimersByTimeAsync(250));
		expect(screen.getByText("Releash: state_restoration")).toBeVisible();
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
		expect(screen.queryByRole("main")).toBeNull();
		await act(async () => resolveRepos(["/current"]));
		expect(completeClientRestoration).toHaveBeenCalledExactlyOnceWith(2);
		await act(() => vi.advanceTimersByTimeAsync(250));
		expect(screen.getByRole("main")).toHaveTextContent("Telemetry: false");
		expect(screen.getByRole("button", { name: "/current" })).toBeVisible();
		expect(screen.queryByText("/old")).toBeNull();
		for (const command of [
			"refresh_workspaces",
			"get_performance_telemetry_enabled",
		]) {
			expect(
				vi.mocked(invokeClient).mock.calls.filter(([name]) => name === command),
			).toHaveLength(2);
		}
	},
);

it("Repository一覧の失敗はWorkspacesの再取得へ委ねて画面全体の操作を再開する", async () => {
	vi.mocked(invokeClient).mockImplementation((command) =>
		command === "refresh_workspaces"
			? Promise.reject(new Error("repository storage unavailable"))
			: Promise.resolve(true),
	);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(completeClientRestoration).toHaveBeenCalledExactlyOnceWith(1);
	expect(screen.getByRole("main")).toBeVisible();
	expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
	expect(
		vi
			.mocked(invoke)
			.mock.calls.some(([command]) => command === "fail_desktop_restoration"),
	).toBe(false);
});

it("復元完了の通知が失敗した場合も理由を報告して再取得できる", async () => {
	vi.mocked(invokeClient).mockImplementation((command) =>
		Promise.resolve(
			command === "refresh_workspaces" ? workspaceListSnapshot() : true,
		),
	);
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

it("設定とWorkspacesは更新中と失敗時も同じ登録一覧を保持し復旧と削除を反映する", async () => {
	const snapshot = (paths: string[]) => ({
		...workspaceListSnapshot(),
		repositories: paths.map((path) => ({
			...workspaceListSnapshot().repositories[0],
			path,
		})),
	});
	let next = Promise.resolve(snapshot(["/old"]));
	vi.mocked(invokeClient).mockImplementation((command) =>
		command === "refresh_workspaces" ? next : Promise.resolve(true),
	);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	fireEvent.click(screen.getByRole("button", { name: "Settings" }));
	const settings = screen.getByRole("region", {
		name: "Registered repositories",
	});
	const subscription = vi
		.mocked(invoke)
		.mock.calls.find(([name]) => name === "subscribe_client_state");
	if (!subscription) throw new Error("Missing state subscription");
	const channel = (
		subscription[1] as { channel: { onmessage: (paths: string[]) => void } }
	).channel;
	await act(async () => channel.onmessage(["/old"]));
	expect(settings).toHaveTextContent("/old");
	let reject!: (error: Error) => void;
	next = new Promise((_, fail) => {
		reject = fail;
	});
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
	});
	expect(settings).toHaveTextContent("/old");
	expect(screen.getByRole("button", { name: "/old" })).toBeVisible();
	await act(async () => {
		reject(new Error("deadline exceeded"));
	});
	expect(settings).toHaveTextContent("/old");
	expect(screen.getByRole("button", { name: "/old" })).toBeVisible();
	next = Promise.resolve(snapshot(["/new", "/other"]));
	await act(async () => channel.onmessage(["/new", "/other"]));
	expect(settings).toHaveTextContent("/new,/other");
	expect(screen.getByRole("button", { name: "/new" })).toBeVisible();
	expect(screen.getByRole("button", { name: "/other" })).toBeVisible();
	expect(screen.queryByRole("button", { name: "/old" })).toBeNull();
	next = Promise.resolve(snapshot([]));
	await act(async () => channel.onmessage([]));
	expect(settings).toBeEmptyDOMElement();
	expect(screen.queryByRole("button", { name: "/new" })).toBeNull();
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.some(([name]) => String(name) === "get_repo_paths"),
	).toBe(false);
});

it("登録一覧の初回取得失敗snapshotでも復元を完了し再取得できる", async () => {
	let next = {
		...workspaceListSnapshot(),
		status: {
			loaded: false,
			error: "read failed" as string | null,
			state: "initialFailed",
		},
		repositories: [] as ReturnType<
			typeof workspaceListSnapshot
		>["repositories"],
	};
	vi.mocked(invokeClient).mockImplementation((command) =>
		Promise.resolve(command === "refresh_workspaces" ? next : true),
	);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(completeClientRestoration).toHaveBeenCalledExactlyOnceWith(1);
	next = workspaceListSnapshot();
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Refresh Workspaces" }));
	});
	expect(screen.getByRole("button", { name: "/repo" })).toBeVisible();
});
