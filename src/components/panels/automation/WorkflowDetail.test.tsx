import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { DiagnosticReport, Workflow } from "@/types/workflow";
import { WorkflowDetail } from "./WorkflowDetail";

const EMPTY_REPORT: DiagnosticReport = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};

function makeWorkflow(overrides?: { input?: string }): Workflow {
	return {
		name: "wf",
		description: "test workflow",
		builtin: false,
		nodes: [
			{
				name: "implement",
				kind: "session",
				session: {
					gate: "auto",
					permission: "edit",
					facets: {
						policy: "coding",
						knowledge: "architecture",
						instruction: "implement",
					},
				},
				artifact: "plan-doc",
				input: overrides?.input,
				rules: [],
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

	it("displays Input row when step declares input", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({
					input: "input-contract",
				})}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(screen.getByText(matchLabel("Input"))).toBeInTheDocument();
		expect(screen.getByText("input-contract")).toBeInTheDocument();
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
