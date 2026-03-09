import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { TimelineEntry } from "@/types/workflow";
import { WorkflowTimeline } from "./WorkflowTimeline";

function makeEntry(overrides?: Partial<TimelineEntry>): TimelineEntry {
	return {
		id: crypto.randomUUID(),
		label: "Test entry",
		status: "pending",
		timestamp: Date.now(),
		...overrides,
	};
}

describe("WorkflowTimeline", () => {
	it("renders empty state when no entries", () => {
		render(<WorkflowTimeline entries={[]} />);
		expect(screen.getByText("No timeline entries")).toBeInTheDocument();
	});

	it("renders entries with labels", () => {
		const entries = [
			makeEntry({ label: "Requirements gathered" }),
			makeEntry({ label: "Plan created" }),
		];

		render(<WorkflowTimeline entries={entries} />);

		expect(screen.getByText("Requirements gathered")).toBeInTheDocument();
		expect(screen.getByText("Plan created")).toBeInTheDocument();
	});

	it("renders status icons for different statuses", () => {
		const entries = [
			makeEntry({ label: "Pending", status: "pending" }),
			makeEntry({ label: "Running", status: "in_progress" }),
			makeEntry({ label: "Done", status: "completed" }),
			makeEntry({ label: "Error", status: "failed" }),
		];

		const { container } = render(<WorkflowTimeline entries={entries} />);

		// Check that all entries are rendered
		expect(screen.getByText("Pending")).toBeInTheDocument();
		expect(screen.getByText("Running")).toBeInTheDocument();
		expect(screen.getByText("Done")).toBeInTheDocument();
		expect(screen.getByText("Error")).toBeInTheDocument();

		// Verify SVG icons are present (one per entry)
		const svgs = container.querySelectorAll("svg");
		expect(svgs.length).toBe(4);
	});
});
