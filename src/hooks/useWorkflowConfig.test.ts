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

	it("should call delete_workflow without refetching", async () => {
		states.publish("workflows", mockWorkflows);
		const { result } = renderHook(() => useWorkflowConfig(true));
		await act(async () => {
			await result.current.deleteWorkflow("my-workflow");
		});
		expect(mocks.invoke).toHaveBeenCalledWith("delete_workflow", {
			name: "my-workflow",
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
	});

	it("should call open_workflow_in_editor", async () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		await act(async () => {
			await result.current.openInEditor("quick-fix");
		});
		expect(mocks.invoke).toHaveBeenCalledWith("open_workflow_in_editor", {
			name: "quick-fix",
		});
	});

	it("購読の失敗をerrorに出す", () => {
		const { result } = renderHook(() => useWorkflowConfig(true));
		act(() => states.fail("workflows", "fetch error"));
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBe("fetch error");
		expect(result.current.workflows).toEqual([]);
	});

	it("should set error when deleteWorkflow fails", async () => {
		mocks.invoke.mockRejectedValue("delete error");
		const { result } = renderHook(() => useWorkflowConfig(true));
		await act(async () => {
			await result.current.deleteWorkflow("my-workflow");
		});
		expect(result.current.error).toBe("delete error");
	});

	it("should set error when openInEditor fails", async () => {
		mocks.invoke.mockRejectedValue("editor error");
		const { result } = renderHook(() => useWorkflowConfig(true));
		await act(async () => {
			await result.current.openInEditor("quick-fix");
		});
		expect(result.current.error).toBe("editor error");
	});
});
