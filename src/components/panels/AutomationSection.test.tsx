import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { useAutomation } from "@/hooks/useAutomation";
import { AutomationSection } from "./AutomationSection";

// Radix UI pointer event polyfills
beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

const EMPTY_REPORT = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};

const SESSION_NODE = {
	name: "step-1",
	kind: "session" as const,
	session: {
		gate: "auto" as const,
		permission: "edit" as const,
		facets: { instruction: "implement" },
	},
	rules: [],
};

function createMockAutomation(
	overrides: Partial<ReturnType<typeof useAutomation>> = {},
): ReturnType<typeof useAutomation> {
	return {
		workflows: [],
		facets: [],
		report: EMPTY_REPORT,
		loading: false,
		error: null,
		setError: vi.fn(),
		selectedWorkflow: null,
		selectedWorkflowSource: null,
		selectedFacetContent: null,
		selectedFacetKey: null,
		selectedFacetKind: null,
		fetchAll: vi.fn(),
		fetchFacets: vi.fn(),
		refreshDiagnostics: vi.fn(),
		selectWorkflow: vi.fn(),
		saveWorkflowSource: vi.fn().mockResolvedValue({
			ok: true,
			workflow: {
				name: "saved-wf",
				description: "",
				builtin: false,
				nodes: [SESSION_NODE],
			},
		}),
		deleteWorkflow: vi.fn(),
		duplicateWorkflow: vi.fn().mockResolvedValue({ ok: true }),
		openWorkflowInEditor: vi.fn(),
		selectFacet: vi.fn(),
		saveFacet: vi.fn().mockResolvedValue({ ok: true }),
		deleteFacet: vi.fn(),
		duplicateFacet: vi.fn().mockResolvedValue({ ok: true }),
		openFacetInEditor: vi.fn(),
		renderFacetPreview: vi.fn().mockResolvedValue("preview"),
		externalChangeDetected: false,
		clearExternalChange: vi.fn(),
		setSelectedWorkflow: vi.fn(),
		setSelectedWorkflowSource: vi.fn(),
		setSelectedFacetContent: vi.fn(),
		setSelectedFacetKey: vi.fn(),
		setSelectedFacetKind: vi.fn(),
		...overrides,
	};
}

describe("AutomationSection", () => {
	it("renders loading state", () => {
		const automation = createMockAutomation({ loading: true });
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("Loading...")).toBeInTheDocument();
	});

	it("renders Workflows and Facets tabs", () => {
		const automation = createMockAutomation();
		render(<AutomationSection automation={automation} />);
		expect(screen.getByRole("tab", { name: "Workflows" })).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Facets" })).toBeInTheDocument();
	});

	it("renders workflow list with builtin badge", () => {
		const automation = createMockAutomation({
			workflows: [
				{
					name: "builtin-workflow",
					description: "Built-in workflow",
					builtin: true,
					is_running: false,
				},
				{
					name: "my-custom",
					description: "Custom workflow",
					builtin: false,
					is_running: false,
				},
			],
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("builtin-workflow")).toBeInTheDocument();
		expect(screen.getByText("my-custom")).toBeInTheDocument();
		expect(screen.getByText("builtin")).toBeInTheDocument();
	});

	it("builtin workflow shows duplicate button but not edit/delete", () => {
		const automation = createMockAutomation({
			workflows: [
				{
					name: "builtin-workflow",
					description: "Built-in",
					builtin: true,
					is_running: false,
				},
			],
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByTitle("Duplicate as custom")).toBeInTheDocument();
		expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
		expect(screen.queryByTitle("Open in editor")).not.toBeInTheDocument();
	});

	it("custom workflow shows edit and delete buttons", () => {
		const automation = createMockAutomation({
			workflows: [
				{
					name: "my-custom",
					description: "Custom",
					builtin: false,
					is_running: false,
				},
			],
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByTitle("Delete")).toBeInTheDocument();
		expect(screen.getByTitle("Open in editor")).toBeInTheDocument();
	});

	it("creates workflow from minimal source without node schema", async () => {
		const user = userEvent.setup();
		const saveWorkflowSource = vi.fn().mockResolvedValue({
			ok: true,
			workflow: {
				name: "new-wf",
				description: "",
				builtin: false,
				nodes: [],
			},
		});
		const selectWorkflow = vi.fn();
		const automation = createMockAutomation({
			saveWorkflowSource,
			selectWorkflow,
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByRole("button"));
		await user.type(screen.getByPlaceholderText("my-workflow"), "new-wf");
		await user.click(screen.getByRole("button", { name: "Create" }));

		await waitFor(() => {
			expect(saveWorkflowSource).toHaveBeenCalledWith(
				'name: new-wf\ndescription: ""\n',
			);
		});
		const source = saveWorkflowSource.mock.calls[0][0];
		expect(source).not.toContain("nodes:");
		expect(source).not.toContain("session:");
		expect(source).not.toContain("permission:");
		expect(source).not.toContain("facets:");
		expect(source).not.toContain("instruction:");
		expect(selectWorkflow).toHaveBeenCalledWith("new-wf");
	});

	it("switches to Facets tab and shows sub-tabs", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation();
		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		expect(screen.getByText("Policy")).toBeInTheDocument();
		expect(screen.getByText("Knowledge")).toBeInTheDocument();
		expect(screen.getByText("Instruction")).toBeInTheDocument();
		expect(screen.queryByText("Contract")).not.toBeInTheDocument();
	});

	it("shows error message when error is set", () => {
		const automation = createMockAutomation({
			error: "Something went wrong",
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("Something went wrong")).toBeInTheDocument();
	});

	it("selectWorkflow is called when clicking a workflow item", async () => {
		const user = userEvent.setup();
		const selectWorkflow = vi.fn();
		const automation = createMockAutomation({
			workflows: [
				{
					name: "test-wf",
					description: "Test",
					builtin: false,
					is_running: false,
				},
			],
			selectWorkflow,
		});
		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("test-wf"));
		expect(selectWorkflow).toHaveBeenCalledWith("test-wf");
	});

	it("workflow detail shows Edit button for custom workflow", () => {
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "my-custom",
				description: "A custom workflow",
				builtin: false,
				nodes: [SESSION_NODE],
			},
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("Edit")).toBeInTheDocument();
	});

	it("workflow detail hides Edit button for builtin workflow", () => {
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "builtin-workflow",
				description: "Builtin",
				builtin: true,
				nodes: [SESSION_NODE],
			},
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.queryByText("Edit")).not.toBeInTheDocument();
	});

	it("facet list shows builtin/custom distinction", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			facets: [
				{
					key: "coding",
					kind: "policies",
					description: "Coding policy",
					builtin: true,
				},
				{
					key: "my-policy",
					kind: "policies",
					description: "Custom policy",
					builtin: false,
				},
			],
		});
		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByText("coding")).toBeInTheDocument();
			expect(screen.getByText("my-policy")).toBeInTheDocument();
		});
	});

	it("shows confirmation when deleting a running workflow", async () => {
		const user = userEvent.setup();
		const deleteWorkflow = vi.fn();
		const automation = createMockAutomation({
			workflows: [
				{
					name: "running-wf",
					description: "Running workflow",
					builtin: false,
					is_running: true,
				},
			],
			deleteWorkflow,
		});
		const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByTitle("Delete"));

		expect(confirmSpy).toHaveBeenCalledWith(expect.stringContaining("実行中"));
		expect(deleteWorkflow).not.toHaveBeenCalled();

		confirmSpy.mockRestore();
	});

	it("shows external change warning when editing a facet and change detected", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedFacetContent: "# My Policy\n\nContent",
			selectedFacetKey: "my-policy",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "my-policy",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			externalChangeDetected: true,
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));
		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(
				screen.getByText(
					"外部でファイルが変更されました。編集中の内容と競合する可能性があります。",
				),
			).toBeInTheDocument();
		});
	});

	it("clicking continue editing clears external change", async () => {
		const user = userEvent.setup();
		const clearExternalChange = vi.fn();
		const automation = createMockAutomation({
			selectedFacetContent: "# My Policy\n\nContent",
			selectedFacetKey: "my-policy",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "my-policy",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			externalChangeDetected: true,
			clearExternalChange,
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));
		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(screen.getByText("編集継続")).toBeInTheDocument();
		});

		await user.click(screen.getByText("編集継続"));

		expect(clearExternalChange).toHaveBeenCalled();
	});

	it("shows facet detail with content and artifact references", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedFacetContent: "Hello {{ request }} world",
			selectedFacetKey: "test-facet",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "test-facet",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getAllByText("test-facet").length).toBeGreaterThanOrEqual(
				1,
			);
		});
		expect(
			screen.getByText(
				(_, element) => element?.textContent === "Hello {{ request }} world",
			),
		).toBeInTheDocument();
		expect(screen.getByText("Edit")).toBeInTheDocument();
	});

	it("shows confirmation when deleting a referenced facet and cancels on deny", async () => {
		const user = userEvent.setup();
		const deleteFacet = vi.fn();
		const automation = createMockAutomation({
			facets: [
				{
					key: "my-policy",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			report: {
				items: [],
				workflow_summaries: {},
				facet_summaries: {},
				facet_usage: {
					"policies/my-policy": [
						{
							workflow_name: "wf-1",
							step_name: "step-1",
							slot: "policy",
						},
					],
				},
			},
			deleteFacet,
		});
		const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByTitle("Delete")).toBeInTheDocument();
		});

		await user.click(screen.getByTitle("Delete"));

		expect(confirmSpy).toHaveBeenCalled();
		expect(deleteFacet).not.toHaveBeenCalled();

		confirmSpy.mockRestore();
	});

	it("facet editor shows preview when Preview button is clicked", async () => {
		const user = userEvent.setup();
		const renderFacetPreview = vi.fn().mockResolvedValue("Hello Alice");
		const automation = createMockAutomation({
			selectedFacetContent: "Hello {{name}}",
			selectedFacetKey: "my-facet",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "my-facet",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			renderFacetPreview,
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByText("Edit")).toBeInTheDocument();
		});

		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(screen.getByText("Preview")).toBeInTheDocument();
		});

		await user.click(screen.getByText("Preview"));

		expect(renderFacetPreview).toHaveBeenCalled();

		await waitFor(() => {
			expect(screen.getByText("Hello Alice")).toBeInTheDocument();
		});
	});

	it("workflow detail Edit opens workflow in external editor", async () => {
		const user = userEvent.setup();
		const openWorkflowInEditor = vi.fn();
		const saveWorkflowSource = vi.fn();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "test-wf",
				description: "Test",
				builtin: false,
				nodes: [SESSION_NODE],
			},
			openWorkflowInEditor,
			saveWorkflowSource,
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Edit"));

		expect(openWorkflowInEditor).toHaveBeenCalledWith("test-wf");
		expect(saveWorkflowSource).not.toHaveBeenCalled();
		expect(
			screen.queryByRole("textbox", { name: "Workflow YAML" }),
		).not.toBeInTheDocument();
	});

	it("workflow detail shows step details when expanded", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "detail-wf",
				description: "Full details",
				builtin: false,
				nodes: [
					{
						name: "complex-step",
						kind: "fanout" as const,
						fanout: {
							parallel_children: [
								{
									name: "child-1",
									facets: { instruction: "child-implement" },
								},
								{
									name: "child-2",
									facets: { instruction: "child-review" },
								},
							],
							aggregate: {
								all_match: "pass",
								// biome-ignore lint/suspicious/noThenProperty: AggregateConfig uses then/else fields
								then: "step-done",
								else: "step-fail",
							},
						},
						artifact: "json-schema",
						inputs: ["step-0"],
						rules: [
							{ type: "next", next: "next-step" },
							{
								type: "loop_guard",
								max_iterations: 3,
								on_exhausted: "fallback-step",
							},
						],
						collect: {
							from: ["step-a", "step-b"],
							reduce: "concat" as const,
						},
					},
				],
			},
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("complex-step"));

		await waitFor(() => {
			expect(screen.getByText("Workflow References")).toBeInTheDocument();
		});
		expect(screen.getByText(/^Artifact:/)).toBeInTheDocument();
		expect(screen.getByText("json-schema")).toBeInTheDocument();
		expect(screen.getByText("Transition Rules")).toBeInTheDocument();
		expect(
			screen.getByText("loop_guard max 3 -> fallback-step"),
		).toBeInTheDocument();
		expect(screen.getByText(/^Inputs:/)).toBeInTheDocument();
		expect(screen.getByText("step-0")).toBeInTheDocument();
		expect(screen.getByText("Fanout Children")).toBeInTheDocument();
		expect(screen.getByText(/child-1/)).toBeInTheDocument();
		expect(screen.getByText(/child-2/)).toBeInTheDocument();
	});

	it("workflow detail shows diagnostics", () => {
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "diag-wf",
				description: "Workflow with diagnostics",
				builtin: false,
				nodes: [SESSION_NODE],
			},
			report: {
				...EMPTY_REPORT,
				items: [
					{
						severity: "error",
						message: "Step references missing facet",
						workflow_name: "diag-wf",
						step_name: "step-1",
					},
					{
						severity: "warning",
						message: "Consider adding artifact schema",
						workflow_name: "diag-wf",
						step_name: "step-1",
					},
				],
			},
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("Diagnostics")).toBeInTheDocument();
		expect(
			screen.getByText("Step references missing facet"),
		).toBeInTheDocument();
		expect(
			screen.getByText("Consider adding artifact schema"),
		).toBeInTheDocument();
	});

	it("facet detail shows Used by section", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedFacetContent: "# My Policy\n\nContent here",
			selectedFacetKey: "my-policy",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "my-policy",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			report: {
				...EMPTY_REPORT,
				facet_usage: {
					"policies/my-policy": [
						{
							workflow_name: "wf-alpha",
							step_name: "step-1",
							slot: "policy",
						},
						{
							workflow_name: "wf-beta",
							step_name: "step-2",
							slot: "policy",
						},
					],
				},
			},
		});
		render(<AutomationSection automation={automation} />);
		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByText("Used by")).toBeInTheDocument();
		});
		expect(screen.getByText(/wf-alpha/)).toBeInTheDocument();
		expect(screen.getByText(/wf-beta/)).toBeInTheDocument();
	});

	it("facet detail shows diagnostics", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedFacetContent: "# My Policy\nContent",
			selectedFacetKey: "test-facet",
			selectedFacetKind: "policy",
			facets: [
				{
					key: "test-facet",
					kind: "policies",
					description: "Test",
					builtin: false,
				},
			],
			report: {
				...EMPTY_REPORT,
				items: [
					{
						severity: "warning",
						message: "Template variable not provided",
						facet_key: "test-facet",
						facet_kind: "policies",
					},
				],
			},
		});
		render(<AutomationSection automation={automation} />);
		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByText("Diagnostics")).toBeInTheDocument();
		});
		expect(
			screen.getByText("Template variable not provided"),
		).toBeInTheDocument();
	});

	it("workflow list shows diagnostic badge", () => {
		const automation = createMockAutomation({
			workflows: [
				{
					name: "wf-with-errors",
					description: "Has errors",
					builtin: false,
					is_running: false,
				},
			],
			report: {
				...EMPTY_REPORT,
				workflow_summaries: {
					"wf-with-errors": {
						error_count: 2,
						warning_count: 1,
						info_count: 0,
					},
				},
			},
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("2")).toBeInTheDocument();
		expect(screen.getByText("1")).toBeInTheDocument();
	});

	it("facet list shows usage count", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			facets: [
				{
					key: "used-policy",
					kind: "policies",
					description: "Used",
					builtin: false,
				},
			],
			report: {
				...EMPTY_REPORT,
				facet_usage: {
					"policies/used-policy": [
						{
							workflow_name: "wf-1",
							step_name: "step-1",
							slot: "policy",
						},
					],
				},
			},
		});
		render(<AutomationSection automation={automation} />);
		await user.click(screen.getByText("Facets"));

		await waitFor(() => {
			expect(screen.getByText("Used by 1")).toBeInTheDocument();
		});
	});
});
