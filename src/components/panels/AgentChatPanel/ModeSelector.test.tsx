import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ModeSelector } from "./ModeSelector";

describe("ModeSelector", () => {
	it("renders all mode buttons", () => {
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={false}
			/>,
		);
		expect(screen.getByText("Code")).toBeInTheDocument();
		expect(screen.getByText("Ask")).toBeInTheDocument();
		expect(screen.getByText("Plan")).toBeInTheDocument();
		expect(screen.getByText("Bypass")).toBeInTheDocument();
	});

	it("calls onModeChange when a different mode is clicked", async () => {
		const onModeChange = vi.fn();
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={onModeChange}
				disabled={false}
			/>,
		);

		await userEvent.click(screen.getByText("Ask"));
		expect(onModeChange).toHaveBeenCalledWith("default");
	});

	it("keeps buttons enabled even when disabled is false (streaming)", () => {
		render(
			<ModeSelector
				mode="acceptEdits"
				onModeChange={vi.fn()}
				disabled={false}
			/>,
		);

		for (const btn of screen.getAllByRole("button")) {
			expect(btn).not.toBeDisabled();
		}
	});

	it("highlights Plan when mode is plan (synced from SDK)", () => {
		render(
			<ModeSelector mode="plan" onModeChange={vi.fn()} disabled={false} />,
		);

		const planButton = screen.getByText("Plan");
		const codeButton = screen.getByText("Code");
		expect(planButton).toHaveAttribute("data-active", "true");
		expect(codeButton).not.toHaveAttribute("data-active", "true");
	});
});
