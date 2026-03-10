import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { TimelineEntry } from "@/types/workflow";
import { PlanPanel } from "./PlanPanel";

vi.mock("@/components/panels/WorkflowTimeline", () => ({
	WorkflowTimeline: ({ entries }: { entries: TimelineEntry[] }) => (
		<div data-testid="workflow-timeline">
			{entries.map((e) => (
				<span key={e.id}>{e.label}</span>
			))}
		</div>
	),
}));

function makeEntry(overrides?: Partial<TimelineEntry>): TimelineEntry {
	return {
		id: crypto.randomUUID(),
		label: "Test entry",
		status: "pending",
		timestamp: Date.now(),
		...overrides,
	};
}

describe("PlanPanel", () => {
	it("renders timeline entries", () => {
		const entries = [makeEntry({ label: "Step 1" })];
		render(<PlanPanel timelineEntries={entries} />);

		expect(screen.getByText("Step 1")).toBeInTheDocument();
	});

	it("renders Complete button when onRequirementsComplete is provided", async () => {
		const user = userEvent.setup();
		const onComplete = vi.fn();

		render(
			<PlanPanel timelineEntries={[]} onRequirementsComplete={onComplete} />,
		);

		const btn = screen.getByRole("button", { name: "Complete" });
		expect(btn).toBeInTheDocument();
		await user.click(btn);
		expect(onComplete).toHaveBeenCalledOnce();
	});

	it("renders Revise button when onRequestRevision is provided", async () => {
		const user = userEvent.setup();
		const onRevise = vi.fn();

		render(<PlanPanel timelineEntries={[]} onRequestRevision={onRevise} />);

		const btn = screen.getByRole("button", { name: "Revise" });
		expect(btn).toBeInTheDocument();
		await user.click(btn);
		expect(onRevise).toHaveBeenCalledOnce();
	});

	it("does not render action buttons when handlers are not provided", () => {
		render(<PlanPanel timelineEntries={[]} />);

		expect(
			screen.queryByRole("button", { name: "Complete" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Revise" }),
		).not.toBeInTheDocument();
	});
});
