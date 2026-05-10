import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAutomation } from "./useAutomation";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockListen = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

describe("useAutomation", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		mockInvoke.mockImplementation((cmd: string) => {
			switch (cmd) {
				case "list_workflows":
					return Promise.resolve([]);
				case "diagnose_all_cmd":
					return Promise.resolve({
						items: [],
						workflow_summaries: {},
						facet_summaries: {},
						facet_usage: {},
					});
				case "list_facet_summaries":
					return Promise.resolve([]);
				case "get_automation_config_dir":
					return Promise.resolve("/mock/config/dir");
				case "start_watching":
					return Promise.resolve(42);
				case "stop_watching":
					return Promise.resolve(undefined);
				default:
					return Promise.resolve(undefined);
			}
		});
	});

	it("fetchAll is called when open=true", async () => {
		renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
			expect(mockInvoke).toHaveBeenCalledWith("diagnose_all_cmd");
		});
	});

	it("fetchAll is not called when open=false", () => {
		renderHook(() => useAutomation(false));
		expect(mockInvoke).not.toHaveBeenCalledWith("list_workflows");
	});

	it("saveWorkflow invokes save_workflow command", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		const wf = {
			name: "test-wf",
			description: "desc",
			builtin: false,
			steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
		};

		let saveResult!: { ok: boolean };
		await act(async () => {
			saveResult = await result.current.saveWorkflow(wf);
		});
		expect(saveResult.ok).toBe(true);
		expect(mockInvoke).toHaveBeenCalledWith("save_workflow", {
			workflow: wf,
			originalName: null,
		});
	});

	it("saveWorkflow with originalName passes it through", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		const wf = {
			name: "new-name",
			description: "desc",
			builtin: false,
			steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
		};

		await act(async () => {
			await result.current.saveWorkflow(wf, "old-name");
		});

		expect(mockInvoke).toHaveBeenCalledWith("save_workflow", {
			workflow: wf,
			originalName: "old-name",
		});
	});

	it("saveWorkflow returns error on failure", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "save_workflow") {
				return Promise.reject("Save failed");
			}
			if (cmd === "list_workflows") return Promise.resolve([]);
			if (cmd === "diagnose_all_cmd")
				return Promise.resolve({
					items: [],
					workflow_summaries: {},
					facet_summaries: {},
					facet_usage: {},
				});
			return Promise.resolve(undefined);
		});

		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		const wf = {
			name: "test",
			description: "",
			builtin: false,
			steps: [{ name: "s1", mode: "auto" as const, rules: [] }],
		};

		let saveResult!: { ok: boolean; error?: string };
		await act(async () => {
			saveResult = await result.current.saveWorkflow(wf);
		});
		expect(saveResult.ok).toBe(false);
		expect(saveResult.error).toBe("Save failed");
	});

	it("deleteWorkflow invokes delete_workflow and refetches", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.deleteWorkflow("test-wf");
		});

		expect(mockInvoke).toHaveBeenCalledWith("delete_workflow", {
			name: "test-wf",
		});
	});

	it("duplicateWorkflow invokes duplicate_workflow command", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		let dupResult!: { ok: boolean };
		await act(async () => {
			dupResult = await result.current.duplicateWorkflow("src", "dest");
		});
		expect(dupResult.ok).toBe(true);
		expect(mockInvoke).toHaveBeenCalledWith("duplicate_workflow", {
			sourceName: "src",
			newName: "dest",
		});
	});

	it("saveFacet invokes save_facet with isNew parameter", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.saveFacet("policy", "my-policy", "content", true);
		});

		expect(mockInvoke).toHaveBeenCalledWith("save_facet", {
			kind: "policy",
			key: "my-policy",
			content: "content",
			isNew: true,
		});
	});

	it("saveFacet without isNew passes null", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.saveFacet("policy", "my-policy", "content");
		});

		expect(mockInvoke).toHaveBeenCalledWith("save_facet", {
			kind: "policy",
			key: "my-policy",
			content: "content",
			isNew: null,
		});
	});

	it("file-change event listener is registered when open", async () => {
		renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"file-change",
				expect.any(Function),
			);
		});
	});

	it("start_watching is called for automation config dir when open", async () => {
		renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_automation_config_dir");
			expect(mockInvoke).toHaveBeenCalledWith("start_watching", {
				path: "/mock/config/dir",
			});
		});
	});

	it("file-change event listener is cleaned up on unmount", async () => {
		const unlisten = vi.fn();
		mockListen.mockResolvedValue(unlisten);

		const { unmount } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalled();
		});

		unmount();

		await waitFor(() => {
			expect(unlisten).toHaveBeenCalled();
		});
	});

	it("selectWorkflow invokes get_workflow", async () => {
		const mockWorkflow = {
			name: "test",
			description: "desc",
			builtin: false,
			steps: [],
		};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_workflow") return Promise.resolve(mockWorkflow);
			if (cmd === "list_workflows") return Promise.resolve([]);
			if (cmd === "diagnose_all_cmd")
				return Promise.resolve({
					items: [],
					workflow_summaries: {},
					facet_summaries: {},
					facet_usage: {},
				});
			return Promise.resolve(undefined);
		});

		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.selectWorkflow("test");
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_workflow", { name: "test" });
		expect(result.current.selectedWorkflow).toEqual(mockWorkflow);
	});

	it("deleteFacet invokes delete_facet and refreshes", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.deleteFacet("policy", "my-policy");
		});

		expect(mockInvoke).toHaveBeenCalledWith("delete_facet", {
			kind: "policy",
			key: "my-policy",
		});
	});
});
