import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import * as client from "@/lib/client";
import { invokeClient } from "@/lib/client";
import { workspaceListSnapshot } from "@/test/workspaceList";
import App from "./App";

vi.mock("react-resizable-panels", () => {
	const Panel = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel">{children}</div>
	);
	const Group = ({ children }: { children?: React.ReactNode }) => (
		<div data-testid="panel-group">{children}</div>
	);
	const Separator = () => <div data-testid="separator" />;
	return { Panel, Group, Separator };
});

vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: () => null,
}));

vi.mock("@/components/panels/ReviewPanel", () => ({ ReviewPanel: () => null }));

const mockInvoke = vi.mocked(invokeClient);
beforeEach(() => {
	mockInvoke.mockClear();
	vi.mocked(client.firstState).mockClear();
	localStorage.clear();
	vi.mocked(invoke).mockImplementation(async (command) => {
		if (command === "check_desktop_update") return null;

		return { type: "ready" };
	});
	mockInvoke.mockImplementation(() =>
		Promise.reject(new Error("not in a git repo")),
	);
});

describe("App", () => {
	it.each(["not_sent", "unknown"] as const)(
		"通信状態%sと再接続メッセージを画面に表示しない",
		async (state) => {
			const status = vi
				.spyOn(client, "invokeClient")
				.mockRejectedValue(new Error(state));
			render(
				<TooltipProvider>
					<App />
				</TooltipProvider>,
			);
			await screen.findByText(
				"Select a worktree from the sidebar to start working",
			);
			expect(
				screen.queryByText(/通信状態を確認|再接続|操作結果を確認|未実行/),
			).not.toBeInTheDocument();
			expect(
				screen.queryByRole("button", { name: "元の操作の結果を確認" }),
			).not.toBeInTheDocument();
			status.mockRestore();
		},
	);

	it("renders layout with empty state message", async () => {
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		await waitFor(() => {
			expect(
				screen.getByText("Select a worktree from the sidebar to start working"),
			).toBeInTheDocument();
		});
	});

	it("Repository一覧の初回取得失敗でも画面を表示し更新を操作できる", async () => {
		vi.mocked(invoke).mockClear();
		const subscribe = vi.mocked(client.subscribeState).getMockImplementation();
		vi.mocked(client.subscribeState).mockImplementation(
			(target, receive, error) => {
				if (target === "workspaces") {
					error?.(new Error("repository unavailable"));
					return () => {};
				}
				return subscribe?.(target, receive, error) ?? (() => {});
			},
		);
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		expect(
			await screen.findByRole("button", { name: "Refresh Workspaces" }),
		).toBeEnabled();
		if (subscribe)
			vi.mocked(client.subscribeState).mockImplementation(subscribe);
	});

	it("reads performance telemetry from the subscription without writing localStorage to Rust on startup", async () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: true }),
		);
		const subscribe = vi.mocked(client.subscribeState).getMockImplementation();
		const received = vi.fn();
		vi.mocked(client.subscribeState).mockImplementation(
			(target, receive, error) => {
				if (target === "desktop-settings") {
					received();
					receive({
						closeToTray: true,
						startMinimized: false,
						crashReporting: true,
						performanceTelemetry: false,
						autoLaunch: false,
					} as never);
					return () => {};
				}
				return subscribe?.(target, receive, error) ?? (() => {});
			},
		);

		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);

		await waitFor(() => expect(received).toHaveBeenCalled());
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"update_performance_telemetry",
			{ enabled: true },
		);
		if (subscribe)
			vi.mocked(client.subscribeState).mockImplementation(subscribe);
	});
});

it.each([true, false])(
	"Repository追加のリポジトリ判定=%sで解決済みルートまたは選択パスを開く",
	async (success) => {
		const user = userEvent.setup();
		const selected = "/chosen/worktree/subdir";
		const subscribe = vi.mocked(client.subscribeState).getMockImplementation();
		vi.mocked(client.subscribeState).mockImplementation(
			(target, receive, error) => {
				if (target === "workspaces") {
					receive(workspaceListSnapshot() as never);
					return () => {};
				}
				return subscribe?.(target, receive, error) ?? (() => {});
			},
		);
		vi.mocked(open).mockResolvedValue(selected);
		mockInvoke.mockImplementation(async (command) => {
			if (command === "find_repository_root")
				return (success ? "/resolved/repository" : null) as never;
			if (command === "add_repo_path") return undefined as never;
			return null as never;
		});
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		await user.click(
			await screen.findByRole("button", { name: "Add Repository" }),
		);
		await waitFor(() =>
			expect(mockInvoke).toHaveBeenCalledWith("find_repository_root", {
				path: selected,
			}),
		);
		if (subscribe)
			vi.mocked(client.subscribeState).mockImplementation(subscribe);
		if (success) {
			await waitFor(() =>
				expect(mockInvoke).toHaveBeenCalledWith("add_repo_path", {
					path: "/resolved/repository",
				}),
			);
			expect(
				screen.queryByTestId(`worktree-pane-${selected}`),
			).not.toBeInTheDocument();
		} else {
			expect(
				await screen.findByTestId(`worktree-pane-${selected}`),
			).toBeInTheDocument();
			expect(mockInvoke).not.toHaveBeenCalledWith(
				"add_repo_path",
				expect.anything(),
			);
		}
	},
);

it("リポジトリ追加の読取失敗を表示し普通のタブを開かない", async () => {
	const user = userEvent.setup();
	const selected = "/chosen/unreadable";
	vi.mocked(open).mockResolvedValue(selected);
	mockInvoke.mockImplementation(async (command) => {
		if (command === "find_repository_root")
			throw new Error("repository denied");
		return null as never;
	});
	render(
		<TooltipProvider>
			<App />
		</TooltipProvider>,
	);
	await user.click(
		await screen.findByRole("button", { name: "Add Repository" }),
	);
	expect(await screen.findByRole("alert")).toHaveTextContent(
		"repository denied",
	);
	expect(
		screen.queryByTestId(`worktree-pane-${selected}`),
	).not.toBeInTheDocument();
	expect(mockInvoke).not.toHaveBeenCalledWith(
		"add_repo_path",
		expect.anything(),
	);
});
