import { describe, expect, it } from "vitest";
import { generateIssueBranchName } from "./issueBranch";

describe("generateIssueBranchName", () => {
	it("generates branch name with issue number", () => {
		expect(generateIssueBranchName(305)).toBe("feat/issues/305");
	});

	it("generates branch name for single digit issue", () => {
		expect(generateIssueBranchName(1)).toBe("feat/issues/1");
	});

	it("generates branch name for large issue number", () => {
		expect(generateIssueBranchName(9999)).toBe("feat/issues/9999");
	});
});
