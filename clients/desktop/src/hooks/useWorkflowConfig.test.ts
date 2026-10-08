import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useWorkflowConfig } from "./useWorkflowConfig";

const states = stateSubscriptions();
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

const mockWorkflows = [
	{
		name: "quick-fix",
		description: "Quick fix workflow",
		builtin: true,
		sourceFormat: "yaml" as const,
		is_running: false,
	},
];

describe("useWorkflowConfig", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		mocks.invoke.mockResolvedValue(undefined);
	});

	it("一覧は購読から届き単発取得を行わない", () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		expect(result.current.loading).toBe(true);
		act(() => states.publish("workflows", mockWorkflows));
		expect(result.current.loading).toBe(false);
		expect(result.current.workflows).toEqual(mockWorkflows);
		expect(result.current.error).toBeNull();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	it("閉じている間は購読しない", () => {
		const { result } = renderHook(() => useWorkflowConfig(false));
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
		expect(result.current.workflows).toEqual([]);
	});

	it("一覧の更新は購読から反映される", () => {
		states.publish("workflows", mockWorkflows);
		const { result } = renderHook(() => useWorkflowConfig(true));
		act(() => states.publish("workflows", []));
		expect(result.current.workflows).toEqual([]);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	it("メニューを開いたとき購読を始める", () => {
		const { result, rerender } = renderHook(
			({ open }) => useWorkflowConfig(open),
			{
				initialProps: { open: false },
			},
		);
		expect(result.current.loading).toBe(false);
		rerender({ open: true });
		expect(result.current.loading).toBe(true);
		expect(states.subscribeState).toHaveBeenCalled();
	});

	it("購読の失敗をerrorに出す", () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		act(() => states.fail("workflows", "fetch error"));
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBe("fetch error");
		expect(result.current.workflows).toEqual([]);
	});

	it("購読の失敗後に値が届けばエラーを消す", () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		act(() => states.fail("workflows", "fetch error"));
		act(() => states.publish("workflows", mockWorkflows));
		expect(result.current.error).toBeNull();
		expect(result.current.workflows).toEqual(mockWorkflows);
	});

	it("購読エラーでも未使用の単発呼び出しを行わない", () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		act(() => states.fail("workflows", "fetch error"));
		expect(result.current.error).toBe("fetch error");
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
