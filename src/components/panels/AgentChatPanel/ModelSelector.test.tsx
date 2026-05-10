import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ModelSelector } from "./ModelSelector";

const models = [
	{ value: "claude-4", displayName: "Claude 4" },
	{ value: "claude-3.5", displayName: "Claude 3.5 Sonnet" },
];

describe("ModelSelector", () => {
	it("shows 'Auto' when no model is selected", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId={null}
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"Auto",
		);
	});

	it("shows selected model name when a model is selected", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toHaveTextContent(
			"Claude 4",
		);
	});

	it("calls onModelChange with model value when a model is selected", async () => {
		const user = userEvent.setup();
		const onModelChange = vi.fn();
		render(
			<ModelSelector
				models={models}
				currentModelId={null}
				onModelChange={onModelChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		await user.click(screen.getByText("Claude 4"));
		expect(onModelChange).toHaveBeenCalledWith("claude-4");
	});

	it("enables trigger when models list is non-empty", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId={null}
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeEnabled();
	});

	it("calls onModelChange with null when Auto is selected", async () => {
		const user = userEvent.setup();
		const onModelChange = vi.fn();
		render(
			<ModelSelector
				models={models}
				currentModelId="claude-4"
				onModelChange={onModelChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("model-selector-trigger"));
		await user.click(screen.getByText("Auto"));
		expect(onModelChange).toHaveBeenCalledWith(null);
	});

	it("disables trigger when disabled is true", () => {
		render(
			<ModelSelector
				models={models}
				currentModelId={null}
				onModelChange={vi.fn()}
				disabled={true}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeDisabled();
	});

	it("disables trigger when models list is empty", () => {
		render(
			<ModelSelector
				models={[]}
				currentModelId={null}
				onModelChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("model-selector-trigger")).toBeDisabled();
	});
});
