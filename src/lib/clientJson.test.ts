import { fromJson, toJson } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import { FailureRecordsSchema } from "@/generated/client_pb";
import { clientJson } from "./clientJson";

describe("失敗記録のwire変換", () => {
	it.each([
		[],
		[
			{
				operation: "workflow_recovery",
				target: "tree",
				classification: "Cancelled",
				message: "",
				count: 1,
				firstObservedMs: 0,
				lastObservedMs: 0,
				requiresAttention: false,
			},
		],
	])("空一覧と既定値を含む記録を往復できる: %j", (...items) => {
		const input = { items, requiresAttention: false, nextOffset: 100 };
		const encoded = clientJson(FailureRecordsSchema, input, true);
		const message = fromJson(FailureRecordsSchema, encoded);
		expect(
			clientJson(
				FailureRecordsSchema,
				toJson(FailureRecordsSchema, message),
				false,
			),
		).toEqual(input);
	});
});
