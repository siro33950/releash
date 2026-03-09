import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { WorkflowPanel } from "./WorkflowPanel";

describe("WorkflowPanel", () => {
	it("renders timeline tab content by default", () => {
		render(
			<WorkflowPanel timelineContent={<div>Timeline here</div>} threads={[]} />,
		);

		expect(screen.getByText("Timeline here")).toBeInTheDocument();
		expect(screen.getByRole("tab", { name: "Timeline" })).toHaveAttribute(
			"data-state",
			"active",
		);
	});

	it("switches to comments tab", async () => {
		const user = userEvent.setup();
		render(
			<WorkflowPanel timelineContent={<div>Timeline here</div>} threads={[]} />,
		);

		await user.click(screen.getByRole("tab", { name: "Comments" }));
		expect(screen.getByRole("tab", { name: "Comments" })).toHaveAttribute(
			"data-state",
			"active",
		);
	});

	it("renders actions slot when provided", () => {
		render(
			<WorkflowPanel
				timelineContent={<div>Timeline</div>}
				actions={<button type="button">Custom Action</button>}
				threads={[]}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Custom Action" }),
		).toBeInTheDocument();
	});

	it("does not render actions container when actions not provided", () => {
		const { container } = render(
			<WorkflowPanel timelineContent={<div>Timeline</div>} threads={[]} />,
		);

		const actionsContainer = container.querySelector(
			".flex.items-center.gap-1",
		);
		expect(actionsContainer).not.toBeInTheDocument();
	});
});
