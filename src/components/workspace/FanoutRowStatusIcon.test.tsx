import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { FanoutRowStatusIcon } from "./FanoutRowStatusIcon";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";

const statuses: WorkspaceNodeStatus[] = [
	"running",
	"paused",
	"failed",
	"waiting",
	"interrupted",
	"aborted",
	"completed",
];

describe("FanoutRowStatusIcon", () => {
	it.each(statuses)("keeps the GitFork icon shape for %s", (status) => {
		const { container } = render(<FanoutRowStatusIcon status={status} />);
		const icon = container.querySelector("svg.lucide-git-fork");

		expect(container.querySelectorAll("svg")).toHaveLength(1);
		expect(icon).toBeInTheDocument();
		expect(icon).toHaveClass(...workflowNodeIconClasses[status].split(" "));
	});

	it("pulses only running and waiting statuses", () => {
		for (const status of statuses) {
			const { container, unmount } = render(
				<FanoutRowStatusIcon status={status} />,
			);
			const icon = container.querySelector("svg.lucide-git-fork");

			if (status === "running" || status === "waiting") {
				expect(icon).toHaveClass("animate-pulse");
			} else {
				expect(icon).not.toHaveClass("animate-pulse");
			}
			unmount();
		}
	});

	it("exposes the backend status as the title", () => {
		render(<FanoutRowStatusIcon status="interrupted" />);

		expect(screen.getByTitle("interrupted")).toBeInTheDocument();
	});
});
