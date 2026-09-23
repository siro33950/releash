import { invoke } from "@tauri-apps/api/core";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import * as client from "@/lib/client";
import { invokeClient } from "@/lib/client";
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

const mockInvoke = vi.mocked(invokeClient);

beforeEach(() => {
	localStorage.clear();
	vi.mocked(invoke).mockImplementation(async (command) =>
		command === "get_daemon_status" ? { phase: "ready" } : { type: "ready" },
	);
	mockInvoke.mockImplementation((cmd: string) => {
		if (cmd === "get_application_startup_outcome") {
			return Promise.resolve({ type: "ready" });
		}
		if (cmd === "get_performance_telemetry_enabled") {
			return Promise.resolve(true);
		}
		return Promise.reject(new Error("not in a git repo"));
	});
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

	it("Repository一覧の初回取得失敗でも画面の復元を完了し更新を操作できる", async () => {
		vi.mocked(invoke).mockClear();
		let restored = false;
		vi.mocked(invoke).mockImplementation(async (command) => {
			if (command === "get_daemon_status")
				return {
					phase: restored ? "ready" : "restoring",
					connectionGeneration: 1,
				};
			return { type: "ready" };
		});
		const complete = vi
			.spyOn(client, "completeClientRestoration")
			.mockImplementation(async () => {
				restored = true;
			});
		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);
		await waitFor(() => expect(complete).toHaveBeenCalledWith(1));
		expect(
			await screen.findByRole("button", { name: "Refresh Workspaces" }),
		).toBeEnabled();
		expect(
			vi
				.mocked(invoke)
				.mock.calls.some(([command]) => command === "fail_desktop_restoration"),
		).toBe(false);
		complete.mockRestore();
	});

	it("reads performance telemetry from Rust without writing localStorage to Rust on startup", async () => {
		localStorage.setItem(
			"releash-settings",
			JSON.stringify({ performanceTelemetry: true }),
		);
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_application_startup_outcome") {
				return Promise.resolve({ type: "ready" });
			}
			if (cmd === "get_performance_telemetry_enabled") {
				return Promise.resolve(false);
			}
			return Promise.reject(new Error("not in a git repo"));
		});

		render(
			<TooltipProvider>
				<App />
			</TooltipProvider>,
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith(
				"get_performance_telemetry_enabled",
			);
		});
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"update_performance_telemetry",
			{ enabled: true },
		);
	});
});
