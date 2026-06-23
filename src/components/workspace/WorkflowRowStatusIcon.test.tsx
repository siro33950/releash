import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkspaceStepStatus } from "@/types/workspace-tree";
import { WorkflowRowStatusIcon } from "./WorkflowRowStatusIcon";
import { workflowStepIconClasses } from "./WorkflowStepStatusIcon";

const statuses: WorkspaceStepStatus[] = [
	"queued",
	"running",
	"failed",
	"error",
	"waiting",
	"aborted",
	"completed",
];

function renderIcon(status: WorkspaceStepStatus) {
	return render(
		<WorkflowRowStatusIcon
			status={status}
			containerClassName="flex size-5 shrink-0 items-center justify-center"
			iconClassName="size-3"
		/>,
	);
}

describe("WorkflowRowStatusIcon", () => {
	it.each(statuses)("keeps the Workflow icon shape for %s", (status) => {
		const { container } = renderIcon(status);

		expect(container.querySelectorAll("svg")).toHaveLength(1);
		expect(container.querySelector("svg.lucide-workflow")).toBeInTheDocument();
	});

	it.each(statuses)("uses the workflow step status color for %s", (status) => {
		const { container } = renderIcon(status);
		const icon = container.querySelector("svg.lucide-workflow");

		expect(icon).toHaveClass(...workflowStepIconClasses[status].split(" "));
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

	it.each([
		"queued",
		"running",
		"completed",
		"error",
	] as const)("exposes %s as the title", (status) => {
		renderIcon(status);

		expect(screen.getByTitle(status)).toBeInTheDocument();
	});

	it("falls back to muted color without pulse for an unknown runtime status", () => {
		const { container } = renderIcon("future-status" as WorkspaceStepStatus);
		const icon = container.querySelector("svg.lucide-workflow");

		expect(screen.getByTitle("future-status")).toBeInTheDocument();
		expect(icon).toHaveClass("text-muted-foreground");
		expect(icon).not.toHaveClass("animate-pulse");
	});
});
