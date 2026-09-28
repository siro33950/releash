import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useAutomation } from "./useAutomation";

const states = stateSubscriptions();
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

const EMPTY_REPORT = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};
const sessionNode = {
	name: "step-1",
	kind: "session" as const,
	session: {
		provider: "claude" as const,
		facets: { instruction: "implement" },
	},
};
const summary = (name: string, sourceFormat: "yaml" | "lua" = "yaml") => ({
	name,
	description: "",
	builtin: false,
	is_running: false,
	sourceFormat,
});
const workflow = (name: string, sourceFormat: "yaml" | "lua" = "yaml") => ({
	name,
	description: "desc",
	builtin: false,
	sourceFormat,
	nodes: [sessionNode],
});

describe("useAutomation", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		mocks.invoke.mockResolvedValue(undefined);
		states.publish("workflows", [
			summary("test"),
			summary("lua-workflow", "lua"),
		]);
		states.publish("diagnostics", EMPTY_REPORT);
	});

	it("一覧と診断は購読から届き単発取得も監視要求も行わない", () => {
		const { result } = renderHook(() => useAutomation(true));
		expect(result.current.loading).toBe(false);
		expect(result.current.workflows.map((item) => item.name)).toEqual([
			"test",
			"lua-workflow",
		]);
		expect(result.current.report).toEqual(EMPTY_REPORT);
		expect(mocks.invoke).not.toHaveBeenCalled();
		act(() => states.publish("workflows", [summary("added")]));
		expect(result.current.workflows.map((item) => item.name)).toEqual([
			"added",
		]);
	});

	it("閉じている間は購読せず届く前はloadingになる", () => {
		renderHook(() => useAutomation(false));
		expect(states.subscribeState).not.toHaveBeenCalled();
		states.clear();
		const { result } = renderHook(() => useAutomation(true));
		expect(result.current.loading).toBe(true);
	});

	it("表示中のfacet種別の一覧を購読する", () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.setFacetKind("policy"));
		act(() =>
			states.publish({ kind: "facets", args: ["policy"] }, [
				{ key: "guide", kind: "policy", description: "", builtin: false },
			]),
		);
		expect(result.current.facets.map((facet) => facet.key)).toEqual(["guide"]);
		act(() => result.current.setFacetKind("knowledge"));
		expect(result.current.facets).toEqual([]);
	});

	it("selectWorkflowは詳細とYAMLソースを購読で受け取る", () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectWorkflow("test"));
		expect(result.current.selectedWorkflowName).toBe("test");
		act(() => {
			states.publish({ kind: "workflow", args: ["test"] }, workflow("test"));
			states.publish(
				{ kind: "workflow-source", args: ["test"] },
				"name: test\nnodes: []\n",
			);
		});
		expect(result.current.selectedWorkflow).toEqual(workflow("test"));
		expect(result.current.selectedWorkflowSource).toBe(
			"name: test\nnodes: []\n",
		);
		expect(result.current.externalChangeDetected).toBe(false);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	it("Luaのworkflowはソースを購読しない", () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectWorkflow("lua-workflow"));
		act(() =>
			states.publish(
				{ kind: "workflow", args: ["lua-workflow"] },
				workflow("lua-workflow", "lua"),
			),
		);
		expect(result.current.selectedWorkflow?.name).toBe("lua-workflow");
		expect(result.current.selectedWorkflowSource).toBeNull();
		expect(states.subscribeState).not.toHaveBeenCalledWith(
			{ kind: "workflow-source", args: ["lua-workflow"] },
			expect.anything(),
			expect.anything(),
		);
	});

	it("読み込めないworkflowはnullとして届きソースと診断は保持する", () => {
		states.publish("workflows", [summary("broken")]);
		states.publish("diagnostics", {
			...EMPTY_REPORT,
			workflow_summaries: { broken: { error_count: 1, info_count: 0 } },
		});
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectWorkflow("broken"));
		act(() => {
			states.publish({ kind: "workflow", args: ["broken"] }, null);
			states.publish(
				{ kind: "workflow-source", args: ["broken"] },
				"name: broken\n",
			);
		});
		expect(result.current.selectedWorkflow).toBeNull();
		expect(result.current.selectedWorkflowName).toBe("broken");
		expect(result.current.selectedWorkflowSource).toBe("name: broken\n");
		expect(result.current.report.workflow_summaries.broken).toEqual({
			error_count: 1,
			info_count: 0,
		});
		act(() =>
			states.publish(
				{ kind: "workflow", args: ["broken"] },
				workflow("broken"),
			),
		);
		expect(result.current.selectedWorkflow?.name).toBe("broken");
	});

	it("選択中ソースの2回目以降の配信を外部変更として検知し自分の保存内容は除く", async () => {
		mocks.invoke.mockImplementation((cmd: string) =>
			Promise.resolve(
				cmd === "save_workflow_source"
					? { ok: true, workflow: workflow("test") }
					: undefined,
			),
		);
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectWorkflow("test"));
		act(() =>
			states.publish({ kind: "workflow-source", args: ["test"] }, "v1"),
		);
		expect(result.current.externalChangeDetected).toBe(false);
		await act(() => result.current.saveWorkflowSource("v2", "test"));
		act(() =>
			states.publish({ kind: "workflow-source", args: ["test"] }, "v2"),
		);
		expect(result.current.externalChangeDetected).toBe(false);
		expect(result.current.selectedWorkflowSource).toBe("v2");
		act(() =>
			states.publish({ kind: "workflow-source", args: ["test"] }, "v3"),
		);
		expect(result.current.externalChangeDetected).toBe(true);
		expect(result.current.selectedWorkflowSource).toBe("v3");
		act(() => result.current.clearExternalChange());
		expect(result.current.externalChangeDetected).toBe(false);
		act(() => result.current.selectWorkflow("lua-workflow"));
		act(() => result.current.selectWorkflow("test"));
		expect(result.current.externalChangeDetected).toBe(false);
	});

	it("saveWorkflowSource invokes save_workflow_source command", async () => {
		const savedWorkflow = workflow("source-wf");
		mocks.invoke.mockImplementation((cmd: string) =>
			Promise.resolve(
				cmd === "save_workflow_source"
					? { ok: true, workflow: savedWorkflow }
					: undefined,
			),
		);
		const { result } = renderHook(() => useAutomation(true));
		let saveResult!: { ok: boolean };
		await act(async () => {
			saveResult = await result.current.saveWorkflowSource(
				"name: source-wf\nnodes: []\n",
				"old-name",
			);
		});
		expect(saveResult.ok).toBe(true);
		expect(mocks.invoke).toHaveBeenCalledWith("save_workflow_source", {
			source: "name: source-wf\nnodes: []\n",
			originalName: "old-name",
		});
		expect(result.current.selectedWorkflow).toEqual(savedWorkflow);
		expect(result.current.selectedWorkflowName).toBe("source-wf");
		expect(result.current.selectedWorkflowSource).toBe(
			"name: source-wf\nnodes: []\n",
		);
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
		mocks.invoke.mockImplementation((cmd: string) =>
			Promise.resolve(
				cmd === "save_workflow_source"
					? { ok: false, error: "workflow_diagnostics", diagnostics }
					: undefined,
			),
		);
		const { result } = renderHook(() => useAutomation(true));
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

	it("deleteWorkflow invokes delete_workflow and clears the selection", async () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectWorkflow("test"));
		await act(async () => {
			await result.current.deleteWorkflow("test");
		});
		expect(mocks.invoke).toHaveBeenCalledWith("delete_workflow", {
			name: "test",
		});
		expect(result.current.selectedWorkflowName).toBeNull();
	});

	it("duplicateWorkflow invokes duplicate_workflow command", async () => {
		const { result } = renderHook(() => useAutomation(true));
		let dupResult!: { ok: boolean };
		await act(async () => {
			dupResult = await result.current.duplicateWorkflow("src", "dest");
		});
		expect(dupResult.ok).toBe(true);
		expect(mocks.invoke).toHaveBeenCalledWith("duplicate_workflow", {
			sourceName: "src",
			newName: "dest",
		});
	});

	it("selectFacetは内容を購読で受け取り外部変更を検知する", async () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectFacet("policy", "guide"));
		act(() =>
			states.publish({ kind: "facet", args: ["policy", "guide"] }, "# guide"),
		);
		expect(result.current.selectedFacetKey).toBe("guide");
		expect(result.current.selectedFacetKind).toBe("policy");
		expect(result.current.selectedFacetContent).toBe("# guide");
		expect(result.current.externalChangeDetected).toBe(false);
		await act(() => result.current.saveFacet("policy", "guide", "# saved"));
		act(() =>
			states.publish({ kind: "facet", args: ["policy", "guide"] }, "# saved"),
		);
		expect(result.current.externalChangeDetected).toBe(false);
		act(() =>
			states.publish({ kind: "facet", args: ["policy", "guide"] }, "# other"),
		);
		expect(result.current.externalChangeDetected).toBe(true);
		expect(result.current.selectedFacetContent).toBe("# other");
		act(() => result.current.clearFacetSelection());
		expect(result.current.selectedFacetKey).toBeNull();
		expect(result.current.selectedFacetContent).toBeNull();
	});

	it("saveFacet invokes save_facet with isNew parameter", async () => {
		const { result } = renderHook(() => useAutomation(true));
		await act(async () => {
			await result.current.saveFacet("policy", "my-policy", "content", true);
		});
		expect(mocks.invoke).toHaveBeenCalledWith("save_facet", {
			kind: "policy",
			key: "my-policy",
			content: "content",
			isNew: true,
		});
	});

	it("saveFacet without isNew passes null", async () => {
		const { result } = renderHook(() => useAutomation(true));
		await act(async () => {
			await result.current.saveFacet("policy", "my-policy", "content");
		});
		expect(mocks.invoke).toHaveBeenCalledWith("save_facet", {
			kind: "policy",
			key: "my-policy",
			content: "content",
			isNew: null,
		});
	});

	it("deleteFacet invokes delete_facet and clears the selection", async () => {
		const { result } = renderHook(() => useAutomation(true));
		act(() => result.current.selectFacet("policy", "my-policy"));
		await act(async () => {
			await result.current.deleteFacet("policy", "my-policy");
		});
		expect(mocks.invoke).toHaveBeenCalledWith("delete_facet", {
			kind: "policy",
			key: "my-policy",
		});
		expect(result.current.selectedFacetKey).toBeNull();
	});

	it("購読の失敗と操作の失敗をerrorに出す", async () => {
		mocks.invoke.mockRejectedValue("delete error");
		const { result } = renderHook(() => useAutomation(true));
		act(() => states.fail("workflows", new Error("stream ended")));
		expect(result.current.error).toBe("stream ended");
		expect(result.current.loading).toBe(false);
		await act(async () => {
			await result.current.deleteWorkflow("test");
		});
		expect(result.current.error).toBe("delete error");
		act(() => result.current.setError(null));
		expect(result.current.error).toBe("stream ended");
	});

	it("閉じると選択を捨てる", async () => {
		const { result, rerender } = renderHook(({ open }) => useAutomation(open), {
			initialProps: { open: true },
		});
		act(() => result.current.selectWorkflow("test"));
		rerender({ open: false });
		await waitFor(() => expect(result.current.selectedWorkflowName).toBeNull());
	});
});
