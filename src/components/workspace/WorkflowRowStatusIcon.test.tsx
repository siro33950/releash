import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";
import { WorkflowRowStatusIcon } from "./WorkflowRowStatusIcon";

const statuses: WorkspaceNodeStatus[] = [
	"queued",
	"running",
	"failed",
	"error",
	"waiting",
	"interrupted",
	"aborted",
	"completed",
];

function renderIcon(status: WorkspaceNodeStatus) {
	return render(<WorkflowRowStatusIcon status={status} />);
}

describe("WorkflowRowStatusIcon", () => {
	it.each(statuses)("keeps the Workflow icon shape for %s", (status) => {
		const { container } = renderIcon(status);

		expect(container.querySelectorAll("svg")).toHaveLength(1);
		expect(container.querySelector("svg.lucide-workflow")).toBeInTheDocument();
	});

	it.each(statuses)("uses the workflow node status color for %s", (status) => {
		const { container } = renderIcon(status);
		const icon = container.querySelector("svg.lucide-workflow");

		expect(icon).toHaveClass(...workflowNodeIconClasses[status].split(" "));
	});

	it("pulses only running and waiting statuses", () => {
		for (const status of statuses) {
			const { container, unmount } = renderIcon(status);
			const icon = container.querySelector("svg.lucide-workflow");

			if (status === "running" || status === "waiting") {
				expect(icon).toHaveClass("animate-pulse");
			} else {
				expect(icon).not.toHaveClass("animate-pulse");
			}

			unmount();
		}
	});

	it.each(["queued", "running", "completed", "error"] as const)(
		"exposes %s as the title",
		(status) => {
			renderIcon(status);

			expect(screen.getByTitle(status)).toBeInTheDocument();
		},
	);

	it("falls back to muted color without pulse for an unknown runtime status", () => {
		const { container } = renderIcon("future-status" as WorkspaceNodeStatus);
		const icon = container.querySelector("svg.lucide-workflow");

		expect(screen.getByTitle("future-status")).toBeInTheDocument();
		expect(icon).toHaveClass("text-muted-foreground");
		expect(icon).not.toHaveClass("animate-pulse");
	});
});
