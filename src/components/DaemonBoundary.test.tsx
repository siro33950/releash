import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { showClientError } from "@/lib/clientErrorNotice";
import { DaemonBoundary } from "./DaemonBoundary";

let status: Record<string, unknown>;
let channel:
	| { onmessage?: (status: Record<string, unknown>) => void }
	| undefined;
beforeEach(() => {
	status = { phase: "starting", retryAvailable: false };
	channel = undefined;
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "subscribe_daemon_status") {
			channel = (args as { channel: typeof channel }).channel;
			channel?.onmessage?.(status);
		}
	});
});
afterEach(() => vi.clearAllMocks());
const publish = async () => act(async () => channel?.onmessage?.(status));
it("Readyまで通常画面を作らず切替中は操作を停止する", async () => {
	render(
		<DaemonBoundary>
			<button type="button">Workflows</button>
		</DaemonBoundary>,
	);
	await publish();
	expect(screen.queryByText("Workflows")).toBeNull();
	expect(
		screen.getByRole("heading", { name: "Starting Releash…" }),
	).toBeVisible();
	expect(screen.getByRole("status")).toHaveTextContent(
		"Waiting for the daemon connection.",
	);
	status = { phase: "ready" };
	await publish();
	expect(screen.getByRole("button", { name: "Workflows" })).toBeVisible();
	status = { phase: "stopping" };
	await publish();
	expect(screen.queryByText("Workflows")).toBeNull();
	expect(screen.getByText("Stopping Releash…")).toBeVisible();
	expect(
		screen.queryByRole("button", { name: /Retry (quit|same effect)/ }),
	).toBeNull();
	status = { phase: "installing" };
	await publish();
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
	await publish();
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

it("操作の通信失敗を画面に表示する", async () => {
	status = { phase: "ready" };
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	await publish();
	await act(async () => showClientError(new Error("Connection unavailable")));
	expect(screen.getByRole("alert")).toHaveTextContent("Connection unavailable");
	fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
	expect(screen.queryByRole("alert")).toBeNull();
});

it("unmountごとにシェル状態の購読を停止する", async () => {
	for (let index = 0; index < 3; index++) {
		const view = render(<DaemonBoundary>workbench</DaemonBoundary>);
		await act(async () => {});
		const start = vi
			.mocked(invoke)
			.mock.calls.filter(([command]) => command === "subscribe_daemon_status")
			.slice(-1)[0];
		if (!start) throw new Error("Missing subscription");
		const id = (start[1] as { id: string }).id;
		view.unmount();
		expect(
			vi
				.mocked(invoke)
				.mock.calls.filter(
					([command, args]) =>
						command === "stop_daemon_status_subscription" &&
						(args as { id?: string } | undefined)?.id === id,
				),
		).toHaveLength(1);
	}
});

it("購読開始の完了前にunmountしても同じ購読を一度停止する", async () => {
	let finish!: () => void;
	vi.mocked(invoke).mockImplementation((command) =>
		command === "subscribe_daemon_status"
			? new Promise<void>((resolve) => {
					finish = resolve;
				})
			: Promise.resolve(),
	);
	const view = render(<DaemonBoundary>workbench</DaemonBoundary>);
	const start = vi
		.mocked(invoke)
		.mock.calls.find(([command]) => command === "subscribe_daemon_status");
	if (!start) throw new Error("Missing subscription");
	const id = (start[1] as { id: string }).id;
	view.unmount();
	expect(invoke).not.toHaveBeenCalledWith("stop_daemon_status_subscription", {
		id,
	});
	await act(async () => finish());
	expect(
		vi
			.mocked(invoke)
			.mock.calls.filter(
				([command, args]) =>
					command === "stop_daemon_status_subscription" &&
					(args as { id?: string } | undefined)?.id === id,
			),
	).toHaveLength(1);
});

it.each([false, true])(
	"購読停止の失敗を記録する: 開始中にunmount=%s",
	async (pending) => {
		const error = new Error("stop failed");
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		let finish!: () => void;
		vi.mocked(invoke).mockImplementation((command) => {
			if (command === "subscribe_daemon_status")
				return pending
					? new Promise<void>((resolve) => {
							finish = resolve;
						})
					: Promise.resolve();
			if (command === "stop_daemon_status_subscription")
				return Promise.reject(error);
			return Promise.resolve();
		});
		const view = render(<DaemonBoundary>workbench</DaemonBoundary>);
		if (!pending) await act(async () => {});
		view.unmount();
		if (pending) await act(async () => finish());
		await act(async () => {});
		expect(log).toHaveBeenCalledWith(
			"Failed to stop daemon status subscription",
			error,
		);
		log.mockRestore();
	},
);

it("ウィンドウ未作成でQuitしても終了の判断操作は表示しない", async () => {
	status = { phase: "stopping" };
	render(
		<DaemonBoundary>
			<div>workbench</div>
		</DaemonBoundary>,
	);
	await publish();
	expect(screen.queryByText("workbench")).toBeNull();
	expect(screen.getByText("Stopping Releash…")).toBeVisible();
	expect(
		screen.queryByRole("button", { name: /Retry (quit|same effect)/ }),
	).toBeNull();
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
	await publish();
	const first = screen.getByText("Action");
	expect(first.closest("[inert]")).not.toBeNull();
	expect(screen.queryByRole("button", { name: "Action" })).toBeNull();
	expect(mounted).toHaveBeenCalledTimes(1);
	status = { phase: "ready", connectionGeneration: 1 };
	await publish();
	expect(screen.getByRole("button", { name: "Action" })).toBe(first);
	status = { phase: "restoring", connectionGeneration: 2 };
	await publish();
	expect(screen.getByText("Action")).not.toBe(first);
	expect(mounted).toHaveBeenCalledTimes(2);
	expect(screen.queryByRole("button", { name: "Action" })).toBeNull();
});
