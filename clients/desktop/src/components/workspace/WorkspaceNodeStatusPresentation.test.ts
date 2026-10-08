import { describe, expect, it } from "vitest";
import type { WorkspaceNodeStatusClassification } from "@/types/workspace-tree";
import { isWorkspaceNodePulseStatus } from "./WorkspaceNodeStatusPresentation";

const classifications: WorkspaceNodeStatusClassification[] = [
	"active",
	"attention",
	"idle",
];

describe("Workspace Node status presentation", () => {
	it("pulses only active and attention", () => {
		for (const classification of classifications) {
			expect(isWorkspaceNodePulseStatus(classification)).toBe(
				classification === "active" || classification === "attention",
			);
		}
	});
});
