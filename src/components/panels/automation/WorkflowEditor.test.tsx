import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { Workflow } from "@/types/workflow";
import { WorkflowEditor } from "./WorkflowEditor";

beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

const ALL_FACET_KEYS = {
	policy: [],
	knowledge: [],
	instruction: [],
	output_contract: [],
};

describe("WorkflowEditor permission handling", () => {
	it("does not backfill missing permission when saving an existing draft", async () => {
		const workflow = {
			name: "wf",
			description: "",
			nodes: [
				{ name: "plan", type: "agent", rules: [] },
				{
					name: "fanout",
					type: "agent",
					rules: [],
					permission: "ask",
					parallel_children: [
						{ name: "child-a", type: "agent" },
						{ name: "child-b", type: "agent", permission: "ask" },
					],
				},
			],
		} as unknown as Workflow;

		const onSave = vi.fn().mockResolvedValue({ ok: true });
		const onCancel = vi.fn();

		render(
			<WorkflowEditor
				workflow={workflow}
				allFacetKeys={ALL_FACET_KEYS}
				onSave={onSave}
				onCancel={onCancel}
			/>,
		);

		const user = userEvent.setup();
		await user.click(screen.getByRole("button", { name: "Save" }));

		expect(onSave).toHaveBeenCalledTimes(1);
		const savedWorkflow = onSave.mock.calls[0][0] as Workflow;

		expect(savedWorkflow.nodes[0].permission).toBeUndefined();

		const parallelStep = savedWorkflow.nodes[1];
		expect(parallelStep.permission).toBe("ask");
		expect(parallelStep.parallel_children?.[0].permission).toBeUndefined();
		expect(parallelStep.parallel_children?.[1].permission).toBe("ask");
	});
});
