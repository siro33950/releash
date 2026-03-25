import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ModeSelector } from "./ModeSelector";

describe("ModeSelector", () => {
	it("shows current mode label on trigger", () => {
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Code",
		);
	});

	it("shows Plan label when mode is plan", () => {
		render(
			<ModeSelector mode="plan" onModeChange={vi.fn()} disabled={false} />,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Plan",
		);
	});

	it("calls onModeChange when a different mode is selected from dropdown", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await user.click(screen.getByTestId("mode-selector-trigger"));
		await user.click(screen.getByText("Ask"));
		expect(onModeChange).toHaveBeenCalledWith("default");
	});

	it("disables trigger when disabled is true", () => {
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={true}
			/>,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toBeDisabled();
	});
});
