import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Thread } from "@/types/thread";
import type { TimelineEntry } from "@/types/workflow";
import { PlanPanel } from "./PlanPanel";

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
	it("renders timeline tab by default", () => {
		const entries = [makeEntry({ label: "Step 1" })];
		render(<PlanPanel timelineEntries={entries} threads={[]} />);

		expect(screen.getByText("Step 1")).toBeInTheDocument();
	});

	it("switches to comments tab", async () => {
		const user = userEvent.setup();
		render(<PlanPanel timelineEntries={[]} threads={[]} />);

		await user.click(screen.getByRole("tab", { name: "Comments" }));
		// CommentList renders empty state
		expect(screen.getByRole("tab", { name: "Comments" })).toHaveAttribute(
			"data-state",
			"active",
		);
	});

	it("renders Complete button when onRequirementsComplete is provided", async () => {
		const user = userEvent.setup();
		const onComplete = vi.fn();

		render(
			<PlanPanel
				timelineEntries={[]}
				threads={[]}
				onRequirementsComplete={onComplete}
			/>,
		);

		const btn = screen.getByRole("button", { name: "Complete" });
		expect(btn).toBeInTheDocument();
		await user.click(btn);
		expect(onComplete).toHaveBeenCalledOnce();
	});

	it("renders Revise button when onRequestRevision is provided", async () => {
		const user = userEvent.setup();
		const onRevise = vi.fn();

		render(
			<PlanPanel
				timelineEntries={[]}
				threads={[]}
				onRequestRevision={onRevise}
			/>,
		);

		const btn = screen.getByRole("button", { name: "Revise" });
		expect(btn).toBeInTheDocument();
		await user.click(btn);
		expect(onRevise).toHaveBeenCalledOnce();
	});

	it("does not render action buttons when handlers are not provided", () => {
		render(<PlanPanel timelineEntries={[]} threads={[]} />);

		expect(
			screen.queryByRole("button", { name: "Complete" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Revise" }),
		).not.toBeInTheDocument();
	});

	it("displays thread content in Comments tab", async () => {
		const user = userEvent.setup();
		const thread: Thread = {
			id: "t1",
			filePath: "workflow://plan",
			lineNumber: 5,
			entries: [
				{
					id: "e1",
					content: "This step needs clarification",
					isAi: false,
					createdAt: Date.now(),
				},
			],
			resolved: false,
			createdAt: Date.now(),
		};

		render(<PlanPanel timelineEntries={[]} threads={[thread]} />);

		await user.click(screen.getByRole("tab", { name: "Comments" }));
		expect(screen.getByText("L5")).toBeInTheDocument();
		expect(
			screen.getByText("This step needs clarification"),
		).toBeInTheDocument();
	});

	it("calls onThreadClick when a thread is clicked", async () => {
		const user = userEvent.setup();
		const onThreadClick = vi.fn();
		const thread: Thread = {
			id: "t1",
			filePath: "workflow://plan",
			lineNumber: 5,
			entries: [
				{
					id: "e1",
					content: "Review this",
					isAi: false,
					createdAt: Date.now(),
				},
			],
			resolved: false,
			createdAt: Date.now(),
		};

		render(
			<PlanPanel
				timelineEntries={[]}
				threads={[thread]}
				onThreadClick={onThreadClick}
			/>,
		);

		await user.click(screen.getByRole("tab", { name: "Comments" }));
		await user.click(screen.getByText("Review this"));
		expect(onThreadClick).toHaveBeenCalledWith("workflow://plan", 5);
	});
});
