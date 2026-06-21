import { describe, expect, it } from "vitest";
import { shouldTailFollowMessageChange } from "./ChatSessionView";

describe("shouldTailFollowMessageChange", () => {
	it("does not tail-follow when older messages are prepended", () => {
		expect(
			shouldTailFollowMessageChange(
				["m3", "m4"],
				["m1", "m2", "m3", "m4"],
				true,
			),
		).toBe(false);
	});

	it("tail-follows when new messages are appended", () => {
		expect(
			shouldTailFollowMessageChange(["m1", "m2"], ["m1", "m2", "m3"], false),
		).toBe(true);
	});

	it("tail-follows initial hydration", () => {
		expect(shouldTailFollowMessageChange([], ["m1"], false)).toBe(true);
	});
});
