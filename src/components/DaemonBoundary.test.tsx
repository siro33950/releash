import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DaemonBoundary } from "./DaemonBoundary";

vi.mock("./layout/ApplicationShutdownBanner", () => ({
	ApplicationShutdownBanner: () => <div>Shutdown decisions</div>,
}));
let status: Record<string, unknown>;
beforeEach(() => {
	vi.useFakeTimers();
	status = { phase: "starting", retryAvailable: false };
	vi.mocked(invoke).mockImplementation(async (command) =>
		command === "get_daemon_status" ? status : undefined,
	);
});
afterEach(() => vi.useRealTimers());
it("Readyまで通常画面を作らず切替中は操作を停止する", async () => {
	render(
		<DaemonBoundary>
			<button type="button">Workflows</button>
		</DaemonBoundary>,
	);
	await act(() => vi.advanceTimersByTimeAsync(0));
	expect(screen.queryByText("Workflows")).toBeNull();
	expect(
		screen.getByRole("heading", { name: "Starting Releash…" }),
	).toBeVisible();
	expect(screen.getByRole("status")).toHaveTextContent(
		"Waiting for the daemon connection.",
	);
	status = { phase: "ready" };
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByRole("button", { name: "Workflows" })).toBeVisible();
	status = { phase: "stopping" };
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.queryByText("Workflows")).toBeNull();
	expect(screen.getByText("Shutdown decisions")).toBeVisible();
	status = { phase: "installing" };
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.queryByText("Workflows")).toBeNull();
	expect(screen.getByText("Installing update…")).toBeVisible();
});
it("失敗段階と理由を示してRustへ再試行と終了を渡す", async () => {
	status = {
		phase: "failed",
		stage: "spawn",
		reason: "Executable missing",
		retryAvailable: true,
	};
	render(
		<DaemonBoundary>
			<div>workbench</div>
		</DaemonBoundary>,
	);
	await act(() => vi.advanceTimersByTimeAsync(0));
	expect(screen.getByText("Releash: spawn")).toBeVisible();
	expect(screen.getByText("Executable missing")).toBeVisible();
	await act(async () =>
		fireEvent.click(screen.getByRole("button", { name: "Retry" })),
	);
	expect(invoke).toHaveBeenCalledWith("retry_daemon");
	await act(async () =>
		fireEvent.click(screen.getByRole("button", { name: "Quit" })),
	);
	expect(invoke).toHaveBeenCalledWith("quit_desktop");
});

it("ウィンドウ未作成でQuitした場合も終了の判断操作を表示する", async () => {
	status = { phase: "stopping" };
	render(
		<DaemonBoundary>
			<div>workbench</div>
		</DaemonBoundary>,
	);
	await act(() => vi.advanceTimersByTimeAsync(0));
	expect(screen.queryByText("workbench")).toBeNull();
	expect(screen.getByText("Shutdown decisions")).toBeVisible();
});

it("状態復元中は画面を操作不可にし再接続では状態を読み直す", async () => {
	const mounted = vi.fn();
	function Workbench() {
		useEffect(() => {
			mounted();
		}, []);
		return <button type="button">Action</button>;
	}
	status = { phase: "restoring", connectionGeneration: 1 };
	render(
		<DaemonBoundary>
			<Workbench />
		</DaemonBoundary>,
	);
	await act(() => vi.advanceTimersByTimeAsync(0));
	const first = screen.getByText("Action");
	expect(first.closest("[inert]")).not.toBeNull();
	expect(screen.queryByRole("button", { name: "Action" })).toBeNull();
	expect(mounted).toHaveBeenCalledTimes(1);
	status = { phase: "ready", connectionGeneration: 1 };
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByRole("button", { name: "Action" })).toBe(first);
	status = { phase: "restoring", connectionGeneration: 2 };
	await act(() => vi.advanceTimersByTimeAsync(250));
	expect(screen.getByText("Action")).not.toBe(first);
	expect(mounted).toHaveBeenCalledTimes(2);
	expect(screen.queryByRole("button", { name: "Action" })).toBeNull();
});
