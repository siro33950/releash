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

function makeWorkflow(overrides?: { input_contracts?: string[] }): Workflow {
	return {
		name: "wf",
		description: "test workflow",
		builtin: false,
		nodes: [
			{
				name: "implement",
				type: "agent",
				policy: "coding",
				knowledge: "architecture",
				instruction: "implement",
				output_contract: "plan-doc",
				input_contracts: overrides?.input_contracts,
				rules: [],
			},
		],
	};
}

const matchLabel = (label: string) => (_content: string, el: Element | null) =>
	el?.tagName === "SPAN" && el.textContent === `${label}:`;

describe("WorkflowDetail facet refs row", () => {
	// Gherkin: ワークフロー詳細画面ではファセット参照として Persona を表示しない
	it("displays facet rows (policy/knowledge/instruction/output_contract) and no Persona row", async () => {
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

		// 4種のラベルが表示される（span のテキストは "Policy:" のように : を含む）
		expect(screen.getByText(matchLabel("Policy"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Knowledge"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Instruction"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Output Contract"))).toBeInTheDocument();

		// Persona ラベルは表示されない
		expect(screen.queryByText(matchLabel("Persona"))).not.toBeInTheDocument();
	});

	// Gherkin: step が input_contracts を宣言している場合、Input Contracts 行を `, ` 連結値で表示する
	it("displays Input Contracts row with comma-separated values when step declares input_contracts", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({
					input_contracts: ["spec-file-path", "approved-fix-policy"],
				})}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(screen.getByText(matchLabel("Input Contracts"))).toBeInTheDocument();
		expect(
			screen.getByText("spec-file-path, approved-fix-policy"),
		).toBeInTheDocument();
	});

	// Gherkin: input_contracts が空配列または undefined のときは Input Contracts 行を表示しない
	it("does not display Input Contracts row when input_contracts is empty", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow({ input_contracts: [] })}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(
			screen.queryByText(matchLabel("Input Contracts")),
		).not.toBeInTheDocument();
	});

	it("does not display Input Contracts row when input_contracts is undefined", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowDetail
				workflow={makeWorkflow()}
				report={EMPTY_REPORT}
				onEdit={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("implement"));

		expect(
			screen.queryByText(matchLabel("Input Contracts")),
		).not.toBeInTheDocument();
	});
});
