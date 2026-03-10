import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { TimelineEntry } from "@/types/workflow";
import { ImplementationPanel } from "./ImplementationPanel";

function makeEntry(overrides?: Partial<TimelineEntry>): TimelineEntry {
	return {
		id: crypto.randomUUID(),
		label: "Test entry",
		status: "pending",
		timestamp: Date.now(),
		...overrides,
	};
}

describe("ImplementationPanel", () => {
	it("shows empty state when not started", () => {
		render(<ImplementationPanel timelineEntries={[]} started={false} />);

		expect(
			screen.getByText("Implementation has not started yet"),
		).toBeInTheDocument();
	});

	it("shows timeline when started", () => {
		const entries = [makeEntry({ label: "Building..." })];
		render(<ImplementationPanel timelineEntries={entries} started={true} />);

		expect(screen.getByText("Building...")).toBeInTheDocument();
	});

	it("renders Approve Plan button", async () => {
		const user = userEvent.setup();
		const onApprove = vi.fn();

		render(
			<ImplementationPanel
				timelineEntries={[]}
				started={false}
				onApprovePlan={onApprove}
			/>,
		);

		const btn = screen.getByRole("button", { name: "Approve Plan" });
		await user.click(btn);
		expect(onApprove).toHaveBeenCalledOnce();
	});

	it("renders Approve button", async () => {
		const user = userEvent.setup();
		const onApprove = vi.fn();

		render(
			<ImplementationPanel
				timelineEntries={[]}
				started={true}
				onApprove={onApprove}
			/>,
		);

		const btn = screen.getByRole("button", { name: "Approve" });
		await user.click(btn);
		expect(onApprove).toHaveBeenCalledOnce();
	});

	it("renders Revise button", async () => {
		const user = userEvent.setup();
		const onRevise = vi.fn();

		render(
			<ImplementationPanel
				timelineEntries={[]}
				started={true}
				onRequestRevision={onRevise}
			/>,
		);

		const btn = screen.getByRole("button", { name: "Revise" });
		await user.click(btn);
		expect(onRevise).toHaveBeenCalledOnce();
	});
});
