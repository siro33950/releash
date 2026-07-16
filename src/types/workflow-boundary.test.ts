/// <reference types="node" />

import { readdirSync, readFileSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND_ROOT = resolve(process.cwd(), "src");
const THIS_FILE = resolve(FRONTEND_ROOT, "types/workflow-boundary.test.ts");

const legacyBoundaryPatterns: Array<[label: string, pattern: RegExp]> = [
	["WorkflowRun", /\bWorkflowRun(?:Summary)?\b/],
	["runId", /\brunId\b/],
	["run_id", /\brun_id\b/],
	["WorkflowStateSnapshot", /\bWorkflowStateSnapshot\b/],
	["WorkflowStatePayload", /\bWorkflowStatePayload\b/],
	["StepHistoryEntry", /\bStepHistoryEntry\b/],
	["StepOutput", /\bStepOutput\b/],
	["ParallelStepState", /\bParallelStepState\b/],
	["WorkflowStep", /\bWorkflowStep\w*\b/],
	["workflowStep", /\bworkflowStep\w*\b/],
	["workflow_step", /\bworkflow_step\w*\b/],
	["stepName", /\bstepName\b/],
	["step_name", /\bstep_name\b/],
	["stepId", /\bstepId\b/],
	["step_id", /\bstep_id\b/],
	["stepType", /\bstepType\b/],
	["step_type", /\bstep_type\b/],
	["runIndex", /\brunIndex\b/],
	["run_index", /\brun_index\b/],
	["step tag", /kind:\s*["']step["']/],
	["steps field", /\bsteps\s*:/],
	["old state event", /workflow-state-changed/],
	["old node status event", /workflow-step-status-changed/],
	["old node status command", /sync_worktree_step_statuses/],
	["old node detail command", /get_workspace_workflow_step_detail/],
	["old approval command", /approve_workflow_step/],
	["old restore command", /restore_workspace_workflow_run/],
	["old archive command", /archive_workspace_workflow_run/],
];

function frontendSourceFiles(directory: string): string[] {
	return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
		const path = join(directory, entry.name);
		if (entry.isDirectory()) return frontendSourceFiles(path);
		if (path === THIS_FILE) return [];
		return [".ts", ".tsx"].includes(extname(path)) ? [path] : [];
	});
}

describe("workflow frontend boundary vocabulary", () => {
	it("does not expose retired execution or node terms", () => {
		const violations = frontendSourceFiles(FRONTEND_ROOT).flatMap((path) => {
			const source = readFileSync(path, "utf8");
			return legacyBoundaryPatterns.flatMap(([label, pattern]) =>
				pattern.test(source)
					? [`${path.slice(FRONTEND_ROOT.length + 1)}: ${label}`]
					: [],
			);
		});

		expect(violations).toEqual([]);
	});
});
