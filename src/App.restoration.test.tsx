import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { completeClientRestoration, invokeClient } from "@/lib/client";
import type { AppSettings } from "@/types/settings";
import App from "./App";

vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	listenClient: vi.fn().mockResolvedValue(() => {}),
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
	SettingsModal: () => null,
}));
vi.mock("@/components/layout/ApplicationShutdownBanner", () => ({
	ApplicationShutdownBanner: () => null,
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
	WorkspaceList: ({ repoPaths }: { repoPaths: string[] }) => (
		<div>
			{repoPaths.map((path) => (
				<button type="button" key={path}>
					{path}
				</button>
			))}
		</div>
	),
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

it.each(["get_repo_paths", "get_performance_telemetry_enabled"] as const)(
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
			if (
				command === "get_repo_paths" ||
				command === "get_performance_telemetry_enabled"
			) {
				if (status.connectionGeneration === 1) {
					if (command === failedCommand)
						return Promise.reject(new Error("temporary read failure"));
					return Promise.resolve(
						command === "get_repo_paths" ? ["/old"] : true,
					);
				}
				return command === "get_repo_paths" ? repos : settings;
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
			"get_repo_paths",
			"get_performance_telemetry_enabled",
		]) {
			expect(
				vi.mocked(invokeClient).mock.calls.filter(([name]) => name === command),
			).toHaveLength(2);
		}
	},
);

it("再取得も失敗した場合は理由と再試行と終了を表示し自動で繰り返さない", async () => {
	vi.mocked(invokeClient).mockImplementation((command) =>
		command === "get_repo_paths"
			? Promise.reject(new Error("repository storage unavailable"))
			: Promise.resolve(true),
	);
	await act(async () => {
		render(<App />);
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Retry" }));
	});
	await act(() => vi.advanceTimersByTimeAsync(250));
	await act(() => vi.advanceTimersByTimeAsync(60_000));
	expect(screen.getByRole("status")).toHaveTextContent(
		"Repositories: repository storage unavailable",
	);
	expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
	expect(screen.queryByRole("main")).toBeNull();
	expect(completeClientRestoration).not.toHaveBeenCalled();
	expect(
		vi
			.mocked(invokeClient)
			.mock.calls.filter(([name]) => name === "get_repo_paths"),
	).toHaveLength(2);
	await act(async () => {
		fireEvent.click(screen.getByRole("button", { name: "Quit" }));
	});
	expect(invoke).toHaveBeenCalledWith("quit_desktop");
});

it("復元完了の通知が失敗した場合も理由を報告して再取得できる", async () => {
	vi.mocked(invokeClient).mockImplementation((command) =>
		Promise.resolve(command === "get_repo_paths" ? [] : true),
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
