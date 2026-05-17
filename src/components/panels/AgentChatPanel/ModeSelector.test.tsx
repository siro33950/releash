import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MODES, ModeSelector } from "./ModeSelector";

describe("ModeSelector", () => {
	it("exposes exactly the three abstract permission modes", () => {
		expect(MODES.map((m) => m.value)).toEqual(["readonly", "edit", "full"]);
	});

	it("shows the Edit label when mode is edit", () => {
		render(
			<ModeSelector mode="edit" onModeChange={vi.fn()} disabled={false} />,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Edit",
		);
	});

	it("shows the Read Only label when mode is readonly", () => {
		render(
			<ModeSelector mode="readonly" onModeChange={vi.fn()} disabled={false} />,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Read Only",
		);
	});

	it("shows the Full label when mode is full", () => {
		render(
			<ModeSelector mode="full" onModeChange={vi.fn()} disabled={false} />,
		);
		expect(screen.getByTestId("mode-selector-trigger")).toHaveTextContent(
			"Full",
		);
	});

	it("calls onModeChange when a different mode is selected from dropdown", async () => {
		const user = userEvent.setup();
		const onModeChange = vi.fn();
		render(
			<ModeSelector mode="edit" onModeChange={onModeChange} disabled={false} />,
		);

		await user.click(screen.getByTestId("mode-selector-trigger"));
		await user.click(screen.getByText("Read Only"));
		expect(onModeChange).toHaveBeenCalledWith("readonly");
	});

	it("disables trigger when disabled is true", () => {
		render(<ModeSelector mode="edit" onModeChange={vi.fn()} disabled={true} />);
		expect(screen.getByTestId("mode-selector-trigger")).toBeDisabled();
	});
});
