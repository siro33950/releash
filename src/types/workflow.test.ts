import { describe, expect, it } from "vitest";
import type { WorkflowEvent } from "./workflow";

describe("WorkflowEvent", () => {
	it("accepts failure policy fields on failed run log events", () => {
		const nodeFailed = {
			event: "node_failed",
			run_id: "run-1",
			workflow_name: "review",
			node_name: "implement",
			reason: "startup timeout",
			failure_kind: "startup_timeout",
			retry_count: 2,
			timestampMs: 1000,
		} satisfies WorkflowEvent;

		const runFailed = {
			event: "run_failed",
			run_id: "run-1",
			workflow_name: "review",
			reason: "structured output mismatch",
			failure_kind: "structured_output_mismatch",
			retry_count: 1,
			timestampMs: 1100,
		} satisfies WorkflowEvent;

		expect(nodeFailed.failure_kind).toBe("startup_timeout");
		expect(runFailed.retry_count).toBe(1);
	});

	it("accepts partial failure fields on parallel child completed events", () => {
		const event = {
			event: "parallel_child_completed",
			run_id: "run-1",
			workflow_name: "review",
			parent_node_name: "parallel-review",
			child_node_name: "review-a",
			session_id: "session-1",
			run_index: 0,
			state: "failed",
			failure_kind: "model_refusal",
			failure_disposition: "partial",
			timestampMs: 1200,
		} satisfies WorkflowEvent;

		expect(event.failure_disposition).toBe("partial");
	});

	it("accepts contract repair request projection fields", () => {
		const event = {
			event: "contract_repair_requested",
			run_id: "run-1",
			workflow_name: "review",
			node_name: "implement",
			run_index: 2,
			request_id: "request-1",
			attempt: 1,
			violation_reason: "missing_submit_output",
			timestampMs: 1300,
		} satisfies WorkflowEvent;

		expect(event.run_index).toBe(2);
		expect(event.request_id).toBe("request-1");
	});
});
