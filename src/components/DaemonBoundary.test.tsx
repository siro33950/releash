import { invoke } from "@tauri-apps/api/core";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "@/components/ui/dialog";
import { getConnectionState, onConnectionStateChange } from "@/lib/client";
import { showClientError } from "@/lib/clientErrorNotice";
import { DaemonBoundary } from "./DaemonBoundary";

vi.mock("@/lib/client", async (original) => ({
	...(await original<typeof import("@/lib/client")>()),
	getConnectionState: vi.fn(),
	onConnectionStateChange: vi.fn(),
}));
let connection: ReturnType<typeof getConnectionState>;
let notifyConnection: (() => void) | undefined;

let status: Record<string, unknown>;
let channel:
	| { onmessage?: (status: Record<string, unknown>) => void }
	| undefined;
beforeEach(() => {
	connection = "CONNECTING";
	vi.mocked(getConnectionState).mockImplementation(() => connection);
	vi.mocked(onConnectionStateChange).mockImplementation((listener) => {
		notifyConnection = listener;
		return () => true;
	});
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
	expect(screen.getByText("Workflows").closest("[inert]")).not.toBeNull();
	expect(screen.getByText("Stopping Releash…")).toBeVisible();
	expect(
		screen.queryByRole("button", { name: /Retry (quit|same effect)/ }),
	).toBeNull();
	status = { phase: "installing" };
	await publish();
	expect(screen.getByText("Workflows").closest("[inert]")).not.toBeNull();
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

it("一度表示した画面は全phaseで保持し覆いのあるphaseだけ操作を停止する", async () => {
	const mounted = vi.fn();
	const unmounted = vi.fn();
	function Workbench() {
		const [value, setValue] = useState("");
		useEffect(() => {
			mounted();
			return unmounted;
		}, []);
		return (
			<input
				aria-label="Draft"
				value={value}
				onChange={(event) => setValue(event.target.value)}
			/>
		);
	}
	status = { phase: "ready" };
	render(
		<DaemonBoundary>
			<Workbench />
		</DaemonBoundary>,
	);
	await publish();
	const input = screen.getByLabelText("Draft");
	fireEvent.change(input, { target: { value: "unsaved" } });
	for (const phase of [
		"starting",
		"backoff",
		"ready",
		"failed",
		"stopping",
		"installing",
		"stopped",
		"ready",
	]) {
		status = { phase, retryAvailable: phase === "failed" };
		await publish();
		expect(screen.getByLabelText("Draft")).toBe(input);
		expect(input).toHaveValue("unsaved");
		const overlay = ["failed", "stopping", "installing", "stopped"].includes(
			phase,
		);
		expect(
			screen.queryByRole("region", { name: "Daemon status" }) !== null,
		).toBe(overlay);
		expect(input.closest("[inert]") !== null).toBe(overlay);
		if (phase === "failed")
			expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
	}
	expect(mounted).toHaveBeenCalledTimes(1);
	expect(unmounted).not.toHaveBeenCalled();
});

it("再接続表示は初回READY以降の画面接続状態だけに従う", async () => {
	status = { phase: "ready" };
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	await publish();
	expect(screen.queryByText("再接続中")).toBeNull();
	connection = "READY";
	await act(async () => notifyConnection?.());
	for (const phase of ["starting", "backoff"]) {
		status = { phase };
		await publish();
		expect(screen.queryByText("再接続中")).toBeNull();
		expect(screen.queryByRole("region", { name: "Daemon status" })).toBeNull();
	}
	status = { phase: "ready" };
	await publish();
	for (const phase of [
		"TRANSIENT_FAILURE",
		"CONNECTING",
		"IDLE",
		"SHUTDOWN",
	] as const) {
		connection = phase;
		await act(async () => notifyConnection?.());
		expect(screen.getByText("再接続中")).toBeVisible();
		expect(screen.getByText("workbench")).toBeVisible();
	}
	connection = "READY";
	await act(async () => notifyConnection?.());
	expect(screen.queryByText("再接続中")).toBeNull();
});

it("Failedでも再試行不可ならQuitだけを提示する", async () => {
	status = { phase: "ready" };
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	await publish();
	status = { phase: "failed", retryAvailable: false };
	await publish();
	expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
	expect(screen.getByRole("button", { name: "Quit" })).toBeVisible();
});

it.each(["pointer", "keyboard"] as const)(
	"Portalモーダルの入力を保持しFailedのRetryとQuitを%sで操作できる",
	async (method) => {
		const user = userEvent.setup();
		status = { phase: "ready" };
		function Settings() {
			const [open, setOpen] = useState(true);
			return (
				<Dialog open={open} onOpenChange={setOpen}>
					<DialogContent>
						<DialogTitle>Settings</DialogTitle>
						<DialogDescription>Edit settings</DialogDescription>
						<input aria-label="Settings draft" defaultValue="" />
					</DialogContent>
				</Dialog>
			);
		}
		render(
			<DaemonBoundary>
				<Settings />
			</DaemonBoundary>,
		);
		await publish();
		const input = screen.getByRole("textbox", { name: "Settings draft" });
		await user.type(input, "unsaved settings");
		for (const retryAvailable of [true, false]) {
			status = { phase: "failed", retryAvailable };
			await publish();
			const quit = screen.getByRole("button", { name: "Quit" });
			const retry = screen.queryByRole("button", { name: "Retry" });
			expect(input).toHaveValue("unsaved settings");
			if (method === "pointer") {
				if (retry) await user.click(retry);
				await user.click(quit);
			} else {
				expect(retry ?? quit).toHaveFocus();
				if (retry) {
					await user.keyboard("{Enter}");
					await user.tab();
				}
				expect(quit).toHaveFocus();
				await user.keyboard("{Enter}");
				await user.tab();
				expect(retry ?? quit).toHaveFocus();
			}
			if (retryAvailable) expect(invoke).toHaveBeenCalledWith("retry_daemon");
			expect(invoke).toHaveBeenCalledWith("quit_desktop");
			status = { phase: "ready" };
			await publish();
			expect(screen.getByRole("textbox", { name: "Settings draft" })).toBe(
				input,
			);
		}
		await user.type(input, " resumed");
		expect(input).toHaveValue("unsaved settings resumed");
	},
);
