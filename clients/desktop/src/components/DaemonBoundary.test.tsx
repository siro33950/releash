import { invoke } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import {
	getClient,
	getConnectionState,
	onConnectionStateChange,
	reconnectClient,
} from "@/lib/client";
import { DaemonBoundary } from "./DaemonBoundary";

vi.mock("@/lib/client", () => ({
	getConnectionState: vi.fn(),
	onConnectionStateChange: vi.fn(),
	reconnectClient: vi.fn(),
	getClient: vi.fn(),
}));
let state: ReturnType<typeof getConnectionState>;
let notify: () => void;
beforeEach(() => {
	vi.clearAllMocks();
	state = "READY";
	vi.mocked(getConnectionState).mockImplementation(() => state);
	vi.mocked(onConnectionStateChange).mockImplementation((callback) => {
		notify = callback;
		return () => true;
	});
	vi.mocked(getClient).mockResolvedValue(
		{} as Awaited<ReturnType<typeof getClient>>,
	);
	vi.mocked(invoke).mockResolvedValue(null);
});
it("接続が失われたら操作を止め、利用者の操作だけで起動する", async () => {
	render(
		<DaemonBoundary>
			<button type="button">Workflows</button>
		</DaemonBoundary>,
	);
	expect(screen.getByText("Workflows")).toBeVisible();
	act(() => {
		state = "TRANSIENT_FAILURE";
		notify();
	});
	expect(screen.getByText("Workflows").closest("[inert]")).not.toBeNull();
	expect(screen.getByText("サーバは動いていません")).toBeVisible();
	expect(invoke).not.toHaveBeenCalledWith("start_daemon");
	fireEvent.click(screen.getByRole("button", { name: "サーバを起動" }));
	await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_daemon"));
	await waitFor(() => expect(reconnectClient).toHaveBeenCalled());
	act(() => {
		state = "READY";
		notify();
	});
	expect(screen.getByRole("button", { name: "Workflows" })).toBeVisible();
});
it("古いサーバの版と入れ替え操作を表示し、Quitをシェルへ渡す", async () => {
	state = "TRANSIENT_FAILURE";
	vi.mocked(invoke).mockImplementation(async (command) =>
		command === "get_desktop_connection_failure"
			? {
					message: "サーバが古い（サーバ 1、画面 2）",
					serverOlder: true,
					clientOlder: false,
				}
			: null,
	);
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	expect(
		await screen.findByText("サーバが古い（サーバ 1、画面 2）"),
	).toBeVisible();
	expect(
		screen.getByRole("heading", { name: "サーバに接続できません" }),
	).toBeVisible();
	fireEvent.click(
		screen.getByRole("button", { name: "サーバを停止して起動し直す" }),
	);
	await waitFor(() => expect(invoke).toHaveBeenCalledWith("replace_daemon"));
	await waitFor(() =>
		expect(screen.getByRole("button", { name: "Quit" })).toBeEnabled(),
	);
	fireEvent.click(screen.getByRole("button", { name: "Quit" }));
	await waitFor(() => expect(invoke).toHaveBeenCalledWith("quit_desktop"));
});
it("起動失敗は理由を表示する", async () => {
	state = "TRANSIENT_FAILURE";
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	await act(async () => {});
	vi.mocked(invoke).mockRejectedValue(new Error("起動失敗"));
	fireEvent.click(screen.getByRole("button", { name: "サーバを起動" }));
	expect(await screen.findByRole("alert")).toHaveTextContent("起動失敗");
});

it("画面が古い失敗では文言とQuitだけを表示する", async () => {
	state = "TRANSIENT_FAILURE";
	vi.mocked(invoke).mockImplementation(async (command) =>
		command === "get_desktop_connection_failure"
			? {
					message: "画面が古い（サーバ 2、画面 1）",
					serverOlder: false,
					clientOlder: true,
				}
			: null,
	);
	render(<DaemonBoundary>workbench</DaemonBoundary>);
	expect(
		await screen.findByText("画面が古い（サーバ 2、画面 1）"),
	).toBeVisible();
	expect(
		screen.queryByRole("button", { name: "サーバを起動" }),
	).not.toBeInTheDocument();
	expect(
		screen.queryByRole("button", { name: "サーバを停止して起動し直す" }),
	).not.toBeInTheDocument();
	fireEvent.click(screen.getByRole("button", { name: "Quit" }));
	await waitFor(() => expect(invoke).toHaveBeenCalledWith("quit_desktop"));
	expect(invoke).not.toHaveBeenCalledWith("start_daemon");
	expect(invoke).not.toHaveBeenCalledWith("replace_daemon");
});
