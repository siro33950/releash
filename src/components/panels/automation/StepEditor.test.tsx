import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { NodeDefinition } from "@/types/workflow";
import { StepEditor } from "./StepEditor";

// Radix UI uses pointer events; jsdom doesn't implement them
beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

function makeNodeDefinition(
	overrides: Partial<NodeDefinition> = {},
): NodeDefinition {
	return {
		name: "implement",
		type: "agent",
		rules: [],
		permission: "edit",
		...overrides,
	};
}

function renderEditor(stepOverride: Partial<NodeDefinition> = {}) {
	const onUpdate = vi.fn();
	const step = makeNodeDefinition(stepOverride);
	render(
		<StepEditor
			step={step}
			index={0}
			totalSteps={1}
			allFacetKeys={{
				policy: [],
				knowledge: [],
				instruction: [],
				output_contract: [],
			}}
			allStepNames={["implement"]}
			onUpdate={onUpdate}
			onRemove={vi.fn()}
			onMove={vi.fn()}
		/>,
	);
	return { onUpdate, step };
}

describe("StepEditor permission select", () => {
	it("offers exactly the three abstract permission options without legacy vocabulary", async () => {
		const user = userEvent.setup();
		renderEditor();

		// 展開
		await user.click(screen.getByText("implement"));

		const trigger = screen.getByTestId("step-0-permission");
		await user.click(trigger);

		const listbox = await screen.findByRole("listbox");
		const options = within(listbox).getAllByRole("option");
		expect(options.map((o) => o.textContent)).toEqual(["Ask", "Edit", "Full"]);

		// 旧語彙は提示されない。
		for (const legacy of [
			"acceptEdits",
			"bypassPermissions",
			"plan",
			"default",
		]) {
			expect(within(listbox).queryByText(legacy)).toBeNull();
		}
	});

	it("propagates selection to onUpdate as the abstract value", async () => {
		const user = userEvent.setup();
		const { onUpdate } = renderEditor({ permission: "edit" });

		await user.click(screen.getByText("implement"));
		await user.click(screen.getByTestId("step-0-permission"));
		await user.click(await screen.findByRole("option", { name: "Ask" }));

		expect(onUpdate).toHaveBeenCalled();
		const updater = onUpdate.mock.calls[0][0] as (
			s: NodeDefinition,
		) => NodeDefinition;
		const updated = updater(makeNodeDefinition({ permission: "edit" }));
		expect(updated.permission).toBe("ask");
	});
});

describe("StepEditor type select", () => {
	// [02] Bash 種別は実行系未対応で backend が拒否するため、UI からも選べないこと
	// を担保する（spec 107 行『bash 実行ロジックは追加しない』との UI 側の整合）。
	it("offers only Agent and Approval; Bash is not selectable", async () => {
		const user = userEvent.setup();
		renderEditor();

		await user.click(screen.getByText("implement"));
		await user.click(screen.getByTestId("step-0-type"));

		const listbox = await screen.findByRole("listbox");
		const options = within(listbox).getAllByRole("option");
		expect(options.map((o) => o.textContent)).toEqual(["Agent", "Approval"]);

		expect(within(listbox).queryByText("Bash")).toBeNull();
		expect(within(listbox).queryByRole("option", { name: "Bash" })).toBeNull();
	});
});
