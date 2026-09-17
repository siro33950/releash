import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ClientConnectionBanner } from "./ClientConnectionBanner";

const model = vi.hoisted(() => ({
	status: {
		connected: true,
		message: null as string | null,
		operations: [] as Array<{
			id: string;
			command: string;
			state: string;
			expired?: boolean;
			canQuery?: boolean;
			canDismiss?: boolean;
		}>,
	},
	retry: vi.fn(),
	dismiss: vi.fn(),
}));
vi.mock("@/lib/clientSocket", () => ({
	getClientStatus: () => model.status,
	subscribeClientStatus: () => () => {},
	retryClientOperation: model.retry,
	dismissClientOperation: model.dismiss,
}));
describe("ClientConnectionBanner", () => {
	it("未送信の期限切れを未実行と表示し確認済みにできる", () => {
		model.status = {
			connected: true,
			message: null,
			operations: [
				{
					id: "unsent",
					command: "add_repo_path",
					state: "not_sent",
					expired: true,
				},
			],
		};
		render(<ClientConnectionBanner />);
		expect(screen.getByRole("status")).toHaveTextContent("未実行");
		expect(
			screen.queryByRole("button", { name: "元の操作の結果を確認" }),
		).not.toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "確認済み" }));
		expect(model.dismiss).toHaveBeenCalledWith("unsent");
	});

	it("結果不明を失敗と表示せず元の操作IDで結果確認する", () => {
		model.status = {
			connected: true,
			message: null,
			operations: [
				{ id: "operation", command: "start_workflow", state: "unknown" },
			],
		};
		render(<ClientConnectionBanner />);
		expect(screen.getByRole("status")).toHaveTextContent(
			"操作結果を確認できません",
		);
		fireEvent.click(
			screen.getByRole("button", { name: "元の操作の結果を確認" }),
		);
		expect(model.retry).toHaveBeenCalledWith("operation");
	});
	it("無応答を表示し切断中の結果照会を無効にする", () => {
		model.status = {
			connected: false,
			message: "通信状態を確認できません。再接続しています。",
			operations: [
				{ id: "operation", command: "start_workflow", state: "unknown" },
			],
		};
		render(<ClientConnectionBanner />);
		expect(screen.getByRole("status")).toHaveTextContent(
			"通信状態を確認できません",
		);
		expect(screen.getByRole("button")).toBeDisabled();
	});
});

it("復元した結果不明は照会を提示せず確認済み失敗を表示する", async () => {
	model.status = {
		connected: true,
		message: null,
		operations: [
			{
				id: "restored",
				command: "add_repo_path",
				state: "unknown",
				expired: true,
				canQuery: false,
				canDismiss: true,
			},
		],
	};
	model.dismiss.mockRejectedValueOnce(new Error("disk full"));
	render(<ClientConnectionBanner />);
	expect(
		screen.queryByRole("button", { name: "元の操作の結果を確認" }),
	).not.toBeInTheDocument();
	fireEvent.click(screen.getByRole("button", { name: "確認済み" }));
	await waitFor(() =>
		expect(screen.getByRole("alert")).toHaveTextContent("保存できませんでした"),
	);
	expect(screen.getByRole("status")).toHaveTextContent(
		"操作結果を確認できません",
	);
	fireEvent.click(screen.getByRole("button", { name: "確認済み" }));
	await waitFor(() =>
		expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
	);
	expect(model.dismiss).toHaveBeenLastCalledWith("restored");
});
