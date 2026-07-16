import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	executeWorkflowAction,
	type WorkflowExecutionAction,
} from "./workflowExecutionActions";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
	invoke: invokeMock,
}));

describe("executeWorkflowAction", () => {
	beforeEach(() => {
		invokeMock.mockReset();
		invokeMock.mockResolvedValue(undefined);
	});

	it.each([
		["stop", "stop_workflow"],
		["resume", "resume_workflow"],
		["abort", "abort_workflow"],
	] satisfies [WorkflowExecutionAction, string][])(
		"invokes the %s command through the shared mapping",
		async (action, command) => {
			await executeWorkflowAction(action, "execution-1");

			expect(invokeMock).toHaveBeenCalledWith(command, {
				executionId: "execution-1",
			});
		},
	);

	it("normalizes command errors", async () => {
		invokeMock.mockRejectedValueOnce("denied");

		await expect(
			executeWorkflowAction("resume", "execution-1"),
		).rejects.toThrow("Resume workflow failed: denied");
	});
});
