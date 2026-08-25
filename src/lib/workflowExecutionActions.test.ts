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

	it.each([
		[{ code: "WORKFLOW_UNAVAILABLE", message: "coded denial" }, "coded denial"],
		["plain denial", "plain denial"],
	])("normalizes command errors", async (rejection, expected) => {
		invokeMock.mockRejectedValueOnce(rejection);

		try {
			await executeWorkflowAction("resume", "execution-1");
			expect.unreachable("workflow action should reject");
		} catch (error) {
			expect(error).toBeInstanceOf(Error);
			expect((error as Error).message).toBe(expected);
		}
	});
});
