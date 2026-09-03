import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatusClassification } from "@/types/workspace-tree";
import {
	isWorkspaceNodePulseStatus,
	workflowNodeIconClasses,
} from "./WorkflowNodeStatusIcon";

const classifications: WorkspaceNodeStatusClassification[] = [
	"active",
	"attention",
	"failure",
	"idle",
	"unbound",
];

describe("Workspace Node status presentation", () => {
	it("maps exactly the backend-owned classifications to their colors", () => {
		expect(Object.keys(workflowNodeIconClasses).sort()).toEqual(
			[...classifications].sort(),
		);
		expect(workflowNodeIconClasses).toEqual({
			active: "text-blue-600 dark:text-blue-300",
			attention: "text-yellow-600 dark:text-yellow-300",
			failure: "text-red-600 dark:text-red-300",
			idle: "text-green-600 dark:text-green-300",
			unbound: "text-muted-foreground",
		});
	});

	it("pulses only active and attention", () => {
		for (const classification of classifications) {
			expect(isWorkspaceNodePulseStatus(classification)).toBe(
				classification === "active" || classification === "attention",
			);
		}
	});
});
