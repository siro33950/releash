import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import {
	isWorkspaceNodePulseStatus,
	workflowNodeIconClasses,
} from "./WorkflowNodeStatusIcon";

const statuses: WorkspaceNodeStatus[] = [
	"running",
	"paused",
	"failed",
	"waiting",
	"interrupted",
	"aborted",
	"completed",
];

describe("Workspace Node status presentation", () => {
	it("covers exactly the backend-owned public statuses", () => {
		expect(Object.keys(workflowNodeIconClasses).sort()).toEqual(
			[...statuses].sort(),
		);
	});

	it.each(statuses)("defines a color for %s", (status) => {
		expect(workflowNodeIconClasses[status]).toBeTruthy();
	});

	it("pulses only running and waiting", () => {
		for (const status of statuses) {
			expect(isWorkspaceNodePulseStatus(status)).toBe(
				status === "running" || status === "waiting",
			);
		}
	});
});
