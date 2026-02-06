import { describe, expect, it } from "vitest";
import type { LineComment } from "@/types/comment";
import { formatCommentsForTerminal } from "./formatCommentsForTerminal";

function makeComment(
	overrides: Partial<LineComment> & { filePath: string; lineNumber: number; content: string },
): LineComment {
	return {
		id: "c-1",
		status: "unsent",
		createdAt: Date.now(),
		...overrides,
	};
}

describe("formatCommentsForTerminal", () => {
	it("should return empty string for no comments", () => {
		expect(formatCommentsForTerminal([])).toBe("");
	});

	it("should format a single comment", () => {
		const result = formatCommentsForTerminal([
			makeComment({
				filePath: "/src/App.tsx",
				lineNumber: 42,
				content: "変数名を改善してください",
			}),
		]);
		expect(result).toContain("## Review Comments");
		expect(result).toContain("### App.tsx");
		expect(result).toContain("- L42: 変数名を改善してください");
	});

	it("should group comments by file", () => {
		const result = formatCommentsForTerminal([
			makeComment({
				id: "c-1",
				filePath: "/src/App.tsx",
				lineNumber: 10,
				content: "comment A",
			}),
			makeComment({
				id: "c-2",
				filePath: "/src/hooks/useAuth.ts",
				lineNumber: 5,
				content: "comment B",
			}),
			makeComment({
				id: "c-3",
				filePath: "/src/App.tsx",
				lineNumber: 20,
				content: "comment C",
			}),
		]);
		expect(result).toContain("### App.tsx");
		expect(result).toContain("### useAuth.ts");
		expect(result).toContain("- L10: comment A");
		expect(result).toContain("- L20: comment C");
		expect(result).toContain("- L5: comment B");
	});

	it("should format range comment as L5-12", () => {
		const result = formatCommentsForTerminal([
			makeComment({
				filePath: "/src/App.tsx",
				lineNumber: 5,
				content: "range comment",
				endLine: 12,
			}),
		]);
		expect(result).toContain("- L5-12: range comment");
	});

	it("should sort comments by line number within a file", () => {
		const result = formatCommentsForTerminal([
			makeComment({
				id: "c-1",
				filePath: "/src/App.tsx",
				lineNumber: 50,
				content: "later",
			}),
			makeComment({
				id: "c-2",
				filePath: "/src/App.tsx",
				lineNumber: 10,
				content: "earlier",
			}),
		]);
		const lines = result.split("\n");
		const earlierIdx = lines.findIndex((l) => l.includes("L10"));
		const laterIdx = lines.findIndex((l) => l.includes("L50"));
		expect(earlierIdx).toBeLessThan(laterIdx);
	});
});
