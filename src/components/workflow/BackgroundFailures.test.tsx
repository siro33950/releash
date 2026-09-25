import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStateSubscription } from "@/hooks/useStateSubscription";
import { BackgroundFailures } from "./BackgroundFailures";

vi.mock("@/hooks/useStateSubscription", () => ({
	useStateSubscription: vi.fn(),
}));
describe("BackgroundFailures", () => {
	beforeEach(() => vi.mocked(useStateSubscription).mockReturnValue(undefined));
	it("対象の失敗記録とサーバが決めた要対応を表示する", () => {
		vi.mocked(useStateSubscription).mockReturnValue({
			requiresAttention: true,
			items: [
				{
					operation: "repository_scan",
					target: "/repo",
					classification: "StateRequired",
					message: "状態を修復してください",
					count: 3,
					firstObservedMs: 1000,
					lastObservedMs: 3000,
					requiresAttention: true,
				},
			],
		});
		render(<BackgroundFailures target="/repo" />);
		expect(useStateSubscription).toHaveBeenCalledWith({
			kind: "failures",
			args: ["/repo"],
		});
		expect(screen.getByText("要対応")).toBeInTheDocument();
		expect(screen.getByText("3回")).toBeInTheDocument();
		expect(screen.getByText("状態を修復してください")).toBeInTheDocument();
	});
	it("記録がなければ表示を追加しない", () => {
		const { container } = render(<BackgroundFailures target="/repo" />);
		expect(container).toBeEmptyDOMElement();
	});
	it("取消の記録は要対応として表示しない", () => {
		vi.mocked(useStateSubscription).mockReturnValue({
			requiresAttention: false,
			items: [
				{
					operation: "repository_scan",
					target: "/repo",
					classification: "Cancelled",
					message: "取消",
					count: 1,
					firstObservedMs: 1000,
					lastObservedMs: 1000,
					requiresAttention: false,
				},
			],
		});
		render(<BackgroundFailures target="/repo" />);
		expect(screen.queryByText("要対応")).not.toBeInTheDocument();
		expect(screen.getByText("取消")).toBeInTheDocument();
	});
});

it("100件より後のページへ進み前のページへ戻れる", async () => {
	const { fireEvent } = await import("@testing-library/react");
	vi.mocked(useStateSubscription).mockReturnValue({
		items: [
			{
				operation: "scan",
				target: "/repo",
				classification: "Internal",
				message: "error",
				count: 1,
				firstObservedMs: 1,
				lastObservedMs: 1,
				requiresAttention: true,
			},
		],
		nextOffset: 100,
		requiresAttention: true,
	});
	render(<BackgroundFailures target="/repo" />);
	fireEvent.click(screen.getByText("次へ"));
	expect(useStateSubscription).toHaveBeenLastCalledWith({
		kind: "failures",
		args: ["/repo", "100"],
	});
	fireEvent.click(screen.getByText("前へ"));
	expect(useStateSubscription).toHaveBeenLastCalledWith({
		kind: "failures",
		args: ["/repo"],
	});
});
