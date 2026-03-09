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
			makeEntry({ label: "Waiting for input", status: "pending" }),
			makeEntry({ label: "Building module", status: "in_progress" }),
			makeEntry({ label: "Tests passed", status: "completed" }),
			makeEntry({ label: "Build failed", status: "failed" }),
		];

		const { container } = render(<WorkflowTimeline entries={entries} />);

		// Check that all entries are rendered
		expect(screen.getByText("Waiting for input")).toBeInTheDocument();
		expect(screen.getByText("Building module")).toBeInTheDocument();
		expect(screen.getByText("Tests passed")).toBeInTheDocument();
		expect(screen.getByText("Build failed")).toBeInTheDocument();

		// Verify SVG icons are present (one per entry)
		const svgs = container.querySelectorAll("svg");
		expect(svgs.length).toBe(4);
	});
});
