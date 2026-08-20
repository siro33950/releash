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

const sessionNode = {
	name: "step-1",
	kind: "session" as const,
	session: {
		provider: "claude" as const,
		facets: { instruction: "implement" },
	},
};

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
				case "get_workflow_source":
					return Promise.resolve("name: test\nnodes: []\n");
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

	it("saveWorkflowSource invokes save_workflow_source command", async () => {
		const savedWorkflow = {
			name: "source-wf",
			description: "desc",
			builtin: false,
			nodes: [sessionNode],
		};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "save_workflow_source")
				return Promise.resolve({ ok: true, workflow: savedWorkflow });
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

		let saveResult!: { ok: boolean };
		await act(async () => {
			saveResult = await result.current.saveWorkflowSource(
				"name: source-wf\nnodes: []\n",
				"old-name",
			);
		});

		expect(saveResult.ok).toBe(true);
		expect(mockInvoke).toHaveBeenCalledWith("save_workflow_source", {
			source: "name: source-wf\nnodes: []\n",
			originalName: "old-name",
		});
		expect(result.current.selectedWorkflow).toEqual(savedWorkflow);
		expect(result.current.selectedWorkflowName).toBe("source-wf");
	});

	it("saveWorkflowSource returns structured diagnostics without stringifying them", async () => {
		const diagnostics = [
			{
				code: "WFT001",
				severity: "error",
				stage: "typecheck",
				span: { start_line: 3, start_col: 5, end_line: 3, end_col: 9 },
				message: "when.on field must be boolean",
				workflow_name: "source-wf",
				field: "rules.when.on",
			},
		];
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "save_workflow_source")
				return Promise.resolve({
					ok: false,
					error: "workflow_diagnostics",
					diagnostics,
				});
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

		let saveResult!: Awaited<
			ReturnType<typeof result.current.saveWorkflowSource>
		>;
		await act(async () => {
			saveResult = await result.current.saveWorkflowSource("bad", "source-wf");
		});

		expect(saveResult).toEqual({
			ok: false,
			error: "workflow_diagnostics",
			diagnostics,
		});
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
			nodes: [],
		};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_workflow") return Promise.resolve(mockWorkflow);
			if (cmd === "get_workflow_source")
				return Promise.resolve("name: test\nnodes: []\n");
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
		expect(mockInvoke).toHaveBeenCalledWith("get_workflow_source", {
			name: "test",
		});
		expect(result.current.selectedWorkflow).toEqual(mockWorkflow);
		expect(result.current.selectedWorkflowName).toBe("test");
		expect(result.current.selectedWorkflowSource).toBe(
			"name: test\nnodes: []\n",
		);
	});

	it("selectWorkflow does not request Lua source", async () => {
		const luaWorkflow = {
			name: "lua-workflow",
			description: "Lua",
			builtin: false,
			sourceFormat: "lua",
			nodes: [],
		};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "list_workflows") {
				return Promise.resolve([
					{
						name: "lua-workflow",
						description: "Lua",
						builtin: false,
						is_running: false,
						sourceFormat: "lua",
					},
				]);
			}
			if (cmd === "get_workflow") return Promise.resolve(luaWorkflow);
			if (cmd === "diagnose_all_cmd") {
				return Promise.resolve({
					items: [],
					workflow_summaries: {},
					facet_summaries: {},
					facet_usage: {},
				});
			}
			return Promise.resolve(undefined);
		});

		const { result } = renderHook(() => useAutomation(true));
		await waitFor(() => expect(result.current.workflows).toHaveLength(1));

		await act(async () => {
			await result.current.selectWorkflow("lua-workflow");
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_workflow", {
			name: "lua-workflow",
		});
		expect(mockInvoke).not.toHaveBeenCalledWith("get_workflow_source", {
			name: "lua-workflow",
		});
		expect(result.current.selectedWorkflowSource).toBeNull();
	});

	it("selectWorkflow keeps source when typed workflow load returns diagnostics", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_workflow") {
				return Promise.reject("workflow_diagnostics: WFS005: legacy field");
			}
			if (cmd === "get_workflow_source") {
				return Promise.resolve("name: broken\nnodes:\n  - type: agent\n");
			}
			if (cmd === "list_workflows") return Promise.resolve([]);
			if (cmd === "diagnose_all_cmd") {
				return Promise.resolve({
					items: [
						{
							code: "WFS005",
							severity: "error",
							stage: "parse_shape",
							span: {
								start_line: 3,
								start_col: 5,
								end_line: 3,
								end_col: 15,
							},
							message: "legacy field",
							workflow_name: "broken",
						},
					],
					workflow_summaries: {
						broken: { error_count: 1, info_count: 0 },
					},
					facet_summaries: {},
					facet_usage: {},
				});
			}
			return Promise.resolve(undefined);
		});

		const { result } = renderHook(() => useAutomation(true));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_workflows");
		});

		await act(async () => {
			await result.current.selectWorkflow("broken");
		});

		expect(result.current.selectedWorkflow).toBeNull();
		expect(result.current.selectedWorkflowName).toBe("broken");
		expect(result.current.selectedWorkflowSource).toBe(
			"name: broken\nnodes:\n  - type: agent\n",
		);
		expect(result.current.report.workflow_summaries.broken).toEqual({
			error_count: 1,
			info_count: 0,
		});
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
