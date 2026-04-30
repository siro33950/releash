import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkflowConfig } from "./useWorkflowConfig";

describe("useWorkflowConfig", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should fetch workflows when open is true", async () => {
		const mockWorkflows = [
			{
				name: "quick-fix",
				description: "Quick fix workflow",
				builtin: true,
			},
		];

		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue(mockWorkflows);

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.workflows).toEqual(mockWorkflows);
		expect(result.current.error).toBeNull();
		expect(vi.mocked(invoke)).toHaveBeenCalledWith("list_workflows");
	});

	it("should not fetch workflows when open is false", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue([]);

		renderHook(() => useWorkflowConfig(false));

		expect(vi.mocked(invoke)).not.toHaveBeenCalledWith("list_workflows");
	});

	it("should call delete_workflow and refresh list", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([]);
				case "delete_workflow":
					return Promise.resolve(undefined);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.deleteWorkflow("my-workflow");
		});

		expect(vi.mocked(invoke)).toHaveBeenCalledWith("delete_workflow", {
			name: "my-workflow",
		});
	});

	it("should call open_workflow_in_editor", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([]);
				case "open_workflow_in_editor":
					return Promise.resolve(undefined);
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.openInEditor("quick-fix");
		});

		expect(vi.mocked(invoke)).toHaveBeenCalledWith("open_workflow_in_editor", {
			name: "quick-fix",
		});
	});

	it("should set error when fetch fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockRejectedValue("fetch error");

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.error).toBe("fetch error");
		expect(result.current.workflows).toEqual([]);
	});

	it("should set error when deleteWorkflow fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([]);
				case "delete_workflow":
					return Promise.reject("delete error");
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.deleteWorkflow("my-workflow");
		});

		expect(result.current.error).toBe("delete error");
	});

	it("should set error when openInEditor fails", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([]);
				case "open_workflow_in_editor":
					return Promise.reject("editor error");
				default:
					return Promise.resolve(null);
			}
		});

		const { result } = renderHook(() => useWorkflowConfig(true));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.openInEditor("quick-fix");
		});

		expect(result.current.error).toBe("editor error");
	});
});
