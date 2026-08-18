import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { DiagnosticReport, WorkflowDefinition } from "@/types/workflow";
import {
	WorkflowDetail,
	WorkflowSourceDiagnosticDetail,
} from "./WorkflowDetail";

const monacoMock = vi.hoisted(() => {
	const model = { dispose: vi.fn() };
	const editor = { dispose: vi.fn() };
	return {
		model,
		module: {
			MarkerSeverity: { Error: 8, Warning: 4, Info: 2 },
			editor: {
				createModel: vi.fn(() => model),
				create: vi.fn(() => editor),
				setModelMarkers: vi.fn(),
			},
		},
	};
});

vi.mock("monaco-editor", () => monacoMock.module);

const EMPTY_REPORT: DiagnosticReport = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};

function makeWorkflow(overrides?: {
	input?: { name: string; contract?: string }[];
	knowledge?: string[];
}): WorkflowDefinition {
	return {
		name: "wf",
		description: "test workflow",
		builtin: false,
		nodes: [
			{
				name: "implement",
				kind: "session",
				session: {
					provider: "claude",
					facets: {
						policy: "coding",
						knowledge: overrides?.knowledge ?? ["architecture"],
						instruction: "implement",
					},
				},
				artifact: "plan-doc",
				input: overrides?.input,
			},
		],
	};
}

const matchLabel = (label: string) => (_content: string, el: Element | null) =>
	el?.tagName === "SPAN" && el.textContent === `${label}:`;

describe("WorkflowDetail facet refs row", () => {
	// Gherkin: ワークフロー詳細画面ではファセット参照として Persona を表示しない
	it("displays workflow reference rows and no Persona row", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow()}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		// 折りたたまれているのでステップを展開
		await user.click(screen.getByText("implement"));

		// span のテキストは "Policy:" のように : を含む
		expect(screen.getByText(matchLabel("Policy"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Knowledge"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Instruction"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Artifact"))).toBeInTheDocument();

		// Persona ラベルは表示されない
		expect(screen.queryByText(matchLabel("Persona"))).not.toBeInTheDocument();
	});

	it("displays Input row when node declares input", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({
					input: [
						{ name: "item", contract: "input-contract" },
						{ name: "spec" },
					],
				})}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(screen.getByText(matchLabel("Input"))).toBeInTheDocument();
		expect(screen.getByText("item: input-contract, spec")).toBeInTheDocument();
	});

	it("displays multiple Knowledge refs in declaration order", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({
					knowledge: ["architecture", "requirements-design"],
				})}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(
			screen.getByText(matchLabel("Knowledge")).parentElement,
		).toHaveTextContent("Knowledge: architecture, requirements-design");
	});

	it("does not display Knowledge row for an empty ref list", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({ knowledge: [] })}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(screen.queryByText(matchLabel("Knowledge"))).not.toBeInTheDocument();
	});

	it("does not display Input row when input is undefined", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow()}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(screen.queryByText(matchLabel("Input"))).not.toBeInTheDocument();
	});
});

describe("WorkflowDetail Monaco diagnostics", () => {
	it("sets workflow diagnostic markers from spanned items only", async () => {
		vi.clearAllMocks();
		const report: DiagnosticReport = {
			items: [
				{
					code: "WFT001",
					severity: "error",
					stage: "typecheck",
					span: {
						start_line: 3,
						start_col: 5,
						end_line: 3,
						end_col: 5,
					},
					message: "when.on field must be boolean",
					workflow_name: "wf",
				},
				{
					code: "WFR003",
					severity: "info",
					stage: "resolve",
					span: {
						start_line: 4,
						start_col: 2,
						end_line: 4,
						end_col: 8,
					},
					message: "unknown reference",
					workflow_name: "wf",
				},
				{
					code: "WFC001",
					severity: "error",
					stage: "control_flow",
					message: "unreachable",
					workflow_name: "wf",
				},
			],
			workflow_summaries: {},
			facet_summaries: {},
			facet_usage: {},
		};

		render(
			<WorkflowSourceDiagnosticDetail
				name="wf"
				report={report}
				source={"name: wf\nnodes: []\n"}
				onEdit={vi.fn()}
			/>,
		);

		await waitFor(() => {
			expect(monacoMock.module.editor.setModelMarkers).toHaveBeenCalled();
		});
		const markerCall = monacoMock.module.editor.setModelMarkers.mock.calls.find(
			([, owner, markers]) =>
				owner === "workflow-diagnostics" && markers.length > 0,
		);
		expect(markerCall).toBeTruthy();
		expect(markerCall?.[0]).toBe(monacoMock.model);
		expect(markerCall?.[1]).toBe("workflow-diagnostics");
		expect(markerCall?.[2]).toEqual([
			{
				severity: 8,
				message: "WFT001: when.on field must be boolean",
				startLineNumber: 3,
				startColumn: 5,
				endLineNumber: 3,
				endColumn: 6,
				code: "WFT001",
			},
			{
				severity: 2,
				message: "WFR003: unknown reference",
				startLineNumber: 4,
				startColumn: 2,
				endLineNumber: 4,
				endColumn: 8,
				code: "WFR003",
			},
		]);
	});
});
