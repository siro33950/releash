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
		selectedFacetContent: null,
		selectedFacetKey: null,
		selectedFacetKind: null,
		fetchAll: vi.fn(),
		fetchFacets: vi.fn(),
		refreshDiagnostics: vi.fn(),
		selectWorkflow: vi.fn(),
		saveWorkflow: vi.fn().mockResolvedValue({ ok: true }),
		deleteWorkflow: vi.fn(),
		duplicateWorkflow: vi.fn().mockResolvedValue({ ok: true }),
		openWorkflowInEditor: vi.fn(),
		selectFacet: vi.fn(),
		saveFacet: vi.fn().mockResolvedValue({ ok: true }),
		deleteFacet: vi.fn(),
		duplicateFacet: vi.fn().mockResolvedValue({ ok: true }),
		openFacetInEditor: vi.fn(),
		renderFacetPreview: vi.fn().mockResolvedValue("preview"),
		loadAllFacetKeys: vi.fn().mockResolvedValue({
			policy: [],
			knowledge: [],
			instruction: [],
			output_contract: [],
		}),
		externalChangeDetected: false,
		clearExternalChange: vi.fn(),
		setSelectedWorkflow: vi.fn(),
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
					name: "spec-driven-development",
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
		expect(screen.getByText("spec-driven-development")).toBeInTheDocument();
		expect(screen.getByText("my-custom")).toBeInTheDocument();
		expect(screen.getByText("builtin")).toBeInTheDocument();
	});

	it("builtin workflow shows duplicate button but not edit/delete", () => {
		const automation = createMockAutomation({
			workflows: [
				{
					name: "spec-driven-development",
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

	it("switches to Facets tab and shows sub-tabs", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation();
		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Facets"));

		expect(screen.getByText("Policy")).toBeInTheDocument();
		expect(screen.getByText("Knowledge")).toBeInTheDocument();
		expect(screen.getByText("Instruction")).toBeInTheDocument();
		expect(screen.getByText("OutputContract")).toBeInTheDocument();
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
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
			},
		});
		render(<AutomationSection automation={automation} />);
		expect(screen.getByText("Edit")).toBeInTheDocument();
	});

	it("workflow detail hides Edit button for builtin workflow", () => {
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "spec-driven-development",
				description: "Builtin",
				builtin: true,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
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

	it("shows external change warning when editing and change detected", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "my-custom",
				description: "A custom workflow",
				builtin: false,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
			},
			externalChangeDetected: true,
			loadAllFacetKeys: vi.fn().mockResolvedValue({
				policy: [],
				knowledge: [],
				instruction: [],
				output_contract: [],
			}),
		});

		render(<AutomationSection automation={automation} />);

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
			selectedWorkflow: {
				name: "my-custom",
				description: "A custom workflow",
				builtin: false,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
			},
			externalChangeDetected: true,
			clearExternalChange,
			loadAllFacetKeys: vi.fn().mockResolvedValue({
				policy: [],
				knowledge: [],
				instruction: [],
				output_contract: [],
			}),
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(screen.getByText("編集継続")).toBeInTheDocument();
		});

		await user.click(screen.getByText("編集継続"));

		expect(clearExternalChange).toHaveBeenCalled();
	});

	it("shows facet detail with content and variables", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedFacetContent: "Hello {{project_name}} world",
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
		expect(screen.getByText("{{project_name}}")).toBeInTheDocument();
		expect(
			screen.getByText("Hello {{project_name}} world"),
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

	it("workflow editor can add a step", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "test-wf",
				description: "Test",
				builtin: false,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
			},
			loadAllFacetKeys: vi.fn().mockResolvedValue({
				policy: [],
				knowledge: [],
				instruction: [],
				output_contract: [],
			}),
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(screen.getByText("Steps (1)")).toBeInTheDocument();
		});

		const stepsHeader = screen.getByText("Steps (1)");
		const addButton = stepsHeader.parentElement?.querySelector(
			"button",
		) as HTMLElement;
		await user.click(addButton);

		await waitFor(() => {
			expect(screen.getByText("Steps (2)")).toBeInTheDocument();
		});
		expect(screen.getByText("step-2")).toBeInTheDocument();
	});

	it("step editor can change mode", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "test-wf",
				description: "Test",
				builtin: false,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
			},
			loadAllFacetKeys: vi.fn().mockResolvedValue({
				policy: [],
				knowledge: [],
				instruction: [],
				output_contract: [],
			}),
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("Edit"));

		await waitFor(() => {
			expect(screen.getByText("step-1")).toBeInTheDocument();
		});

		await user.click(screen.getByText("step-1"));

		await waitFor(() => {
			expect(screen.getByText("Mode")).toBeInTheDocument();
		});

		const modeTrigger = screen.getByText("Auto").closest("button");
		if (!modeTrigger) throw new Error("Mode trigger button not found");
		await user.click(modeTrigger);

		await waitFor(() => {
			expect(
				screen.getByRole("option", { name: "Approval" }),
			).toBeInTheDocument();
		});

		await user.click(screen.getByRole("option", { name: "Approval" }));

		await waitFor(() => {
			expect(screen.getByText("approval")).toBeInTheDocument();
		});
	});

	it("workflow detail shows step details when expanded", async () => {
		const user = userEvent.setup();
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "detail-wf",
				description: "Full details",
				builtin: false,
				steps: [
					{
						name: "complex-step",
						mode: "auto" as const,
						policy: "coding-policy",
						knowledge: "project-docs",
						instruction: "do-thing",
						output_contract: "json-schema",
						rules: [{ match: "pass", next: "next-step" }],
						cycle_guard: { max_iterations: 3 },
						pass_previous_response: true,
						pass_output_from: ["step-0"],
						inline_prompt: "Run the tests",
						collect: {
							from: ["step-a", "step-b"],
							reduce: "concat" as const,
						},
						parallel: [
							{ name: "child-1", mode: "auto" as const },
							{ name: "child-2", mode: "approval" as const },
						],
						aggregate: {
							all_match: "pass",
							// biome-ignore lint/suspicious/noThenProperty: AggregateConfig uses then/else fields
							then: "step-done",
							else: "step-fail",
						},
					},
				],
			},
		});

		render(<AutomationSection automation={automation} />);

		await user.click(screen.getByText("complex-step"));

		await waitFor(() => {
			expect(screen.getByText("Facet References")).toBeInTheDocument();
		});
		expect(screen.getByText("Transition Rules")).toBeInTheDocument();
		expect(screen.getByText("Inline Prompt")).toBeInTheDocument();
		expect(screen.getByText("Run the tests")).toBeInTheDocument();
		expect(
			screen.getByText("Cycle Guard: max 3 iterations"),
		).toBeInTheDocument();
		expect(screen.getByText("Pass previous response: yes")).toBeInTheDocument();
		expect(screen.getByText(/Pass output from: step-0/)).toBeInTheDocument();
		expect(screen.getByText("Parallel Steps")).toBeInTheDocument();
		expect(screen.getByText(/child-1/)).toBeInTheDocument();
		expect(screen.getByText(/child-2/)).toBeInTheDocument();
	});

	it("workflow detail shows diagnostics", () => {
		const automation = createMockAutomation({
			selectedWorkflow: {
				name: "diag-wf",
				description: "Workflow with diagnostics",
				builtin: false,
				steps: [{ name: "step-1", mode: "auto" as const, rules: [] }],
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
						message: "Consider adding output contract",
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
			screen.getByText("Consider adding output contract"),
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
