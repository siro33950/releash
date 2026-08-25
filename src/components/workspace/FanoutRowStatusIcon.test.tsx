import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatusClassification } from "@/types/workspace-tree";
import { FanoutRowStatusIcon } from "./FanoutRowStatusIcon";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";

const classifications: WorkspaceNodeStatusClassification[] = [
	"active",
	"attention",
	"failure",
	"idle",
];

describe("FanoutRowStatusIcon", () => {
	it.each(classifications)(
		"keeps the GitFork icon shape and classification color for %s",
		(status) => {
			const { container } = render(<FanoutRowStatusIcon status={status} />);
			const icon = container.querySelector("svg.lucide-git-fork");

			expect(container.querySelectorAll("svg")).toHaveLength(1);
			expect(icon).toBeInTheDocument();
			expect(icon).toHaveClass(...workflowNodeIconClasses[status].split(" "));
		},
	);

	it("pulses only active and attention classifications", () => {
		for (const status of classifications) {
			const { container, unmount } = render(
				<FanoutRowStatusIcon status={status} />,
			);
			const icon = container.querySelector("svg.lucide-git-fork");

			if (status === "active" || status === "attention") {
				expect(icon).toHaveClass("animate-pulse");
			} else {
				expect(icon).not.toHaveClass("animate-pulse");
			}
			unmount();
		}
	});

	it("exposes the backend status as the title", () => {
		render(<FanoutRowStatusIcon status="attention" />);

		expect(screen.getByTitle("attention")).toBeInTheDocument();
	});
});
