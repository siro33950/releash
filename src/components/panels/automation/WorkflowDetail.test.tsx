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

function makeWorkflow(): Workflow {
	return {
		name: "wf",
		description: "test workflow",
		builtin: false,
		steps: [
			{
				name: "implement",
				mode: "auto",
				policy: "coding",
				knowledge: "architecture",
				instruction: "implement",
				output_contract: "plan-doc",
				rules: [],
			},
		],
	};
}

describe("WorkflowDetail facet refs row", () => {
	// Gherkin: ワークフロー詳細画面ではファセット参照として Persona を表示しない
	it("displays 4 facet rows (policy/knowledge/instruction/output_contract) and no Persona row", async () => {
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
		const matchLabel =
			(label: string) => (_content: string, el: Element | null) =>
				el?.tagName === "SPAN" && el.textContent === `${label}:`;
		expect(screen.getByText(matchLabel("Policy"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Knowledge"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("Instruction"))).toBeInTheDocument();
		expect(screen.getByText(matchLabel("OutputContract"))).toBeInTheDocument();

		// Persona ラベルは表示されない
		expect(screen.queryByText(matchLabel("Persona"))).not.toBeInTheDocument();
	});
});
