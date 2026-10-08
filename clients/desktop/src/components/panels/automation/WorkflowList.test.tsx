import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { WorkflowList } from "./WorkflowList";

it("読めない定義を一覧に残し説明文と区別して失敗欄を表示する", () => {
	render(
		<WorkflowList
			workflows={[
				{
					name: "unreadable",
					description: "",
					builtin: false,
					is_running: false,
					sourceFormat: "yaml",
					readError: "definition could not be read",
				},
				{
					name: "healthy",
					description: "definition could not be read",
					builtin: false,
					is_running: false,
					sourceFormat: "yaml",
				},
			]}
			report={{
				items: [],
				workflow_summaries: {},
				facet_summaries: {},
				facet_usage: {},
			}}
			selectedName={null}
			onSelect={vi.fn()}
			onDelete={vi.fn()}
			onDuplicate={vi.fn()}
			onEdit={vi.fn()}
			onCreate={vi.fn()}
		/>,
	);
	expect(screen.getByText("unreadable")).toBeInTheDocument();
	expect(screen.getByText("healthy")).toBeInTheDocument();
	expect(screen.getAllByRole("alert")).toHaveLength(1);
	expect(screen.getByRole("alert")).toHaveTextContent(
		"definition could not be read",
	);
});
