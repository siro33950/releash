import { describe, expect, it } from "vitest";
import { generateNotionBranchName } from "./notionBranch";

describe("generateNotionBranchName", () => {
	it("returns the value as-is when already valid", () => {
		expect(generateNotionBranchName("feat/add-login")).toBe("feat/add-login");
	});

	it("converts spaces to hyphens", () => {
		expect(generateNotionBranchName("fix login bug")).toBe("fix-login-bug");
	});

	it("removes special characters", () => {
		expect(generateNotionBranchName("feat: add @login!")).toBe(
			"feat-add-login",
		);
	});

	it("collapses consecutive hyphens", () => {
		expect(generateNotionBranchName("fix -- bug")).toBe("fix-bug");
	});

	it("trims leading and trailing hyphens", () => {
		expect(generateNotionBranchName("--fix-bug--")).toBe("fix-bug");
	});

	it("preserves original case", () => {
		expect(generateNotionBranchName("Fix/Login-Bug")).toBe("Fix/Login-Bug");
	});

	it("preserves uppercase prefix like PROJ-123", () => {
		expect(generateNotionBranchName("PROJ-123")).toBe("PROJ-123");
	});

	it("returns fallback for empty string without pageId", () => {
		expect(generateNotionBranchName("")).toBe("notion-task");
	});

	it("returns fallback when only special characters without pageId", () => {
		expect(generateNotionBranchName("!@#$%")).toBe("notion-task");
	});

	it("uses pageId fallback for Japanese-only title", () => {
		expect(
			generateNotionBranchName(
				"機能追加",
				"a1b2c3d4-e5f6-7890-abcd-ef1234567890",
			),
		).toBe("notion/a1b2c3d4");
	});

	it("uses pageId fallback for empty string", () => {
		expect(
			generateNotionBranchName("", "abcdef12-3456-7890-abcd-ef1234567890"),
		).toBe("notion/abcdef12");
	});

	it("strips hyphens from pageId in fallback", () => {
		expect(generateNotionBranchName("!@#", "ab-cd-ef-12-34-56")).toBe(
			"notion/abcdef12",
		);
	});

	it("handles mixed Japanese and ASCII", () => {
		expect(generateNotionBranchName("feat/ログイン-fix")).toBe("feat/-fix");
	});

	it("handles whitespace-only input with pageId", () => {
		expect(
			generateNotionBranchName("   ", "12345678-abcd-efgh-ijkl-mnopqrstuvwx"),
		).toBe("notion/12345678");
	});

	it("handles whitespace-only input without pageId", () => {
		expect(generateNotionBranchName("   ")).toBe("notion-task");
	});

	it("preserves underscores", () => {
		expect(generateNotionBranchName("fix_login_bug")).toBe("fix_login_bug");
	});

	it("preserves slashes for branch paths", () => {
		expect(generateNotionBranchName("feat/issues/123")).toBe("feat/issues/123");
	});

	it("does not use pageId when ASCII sanitization succeeds", () => {
		expect(
			generateNotionBranchName(
				"fix-bug",
				"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
			),
		).toBe("fix-bug");
	});
});
