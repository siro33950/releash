import { describe, expect, it } from "vitest";
import type { LineComment } from "@/types/comment";
import { formatCommentsForTerminal } from "./formatCommentsForTerminal";

function makeComment(
	overrides: Partial<LineComment> & {
		filePath: string;
		lineNumber: number;
		content: string;
	},
): LineComment {
	return {
		id: "c-1",
		status: "unsent",
		createdAt: Date.now(),
		...overrides,
	};
}

const ROOT = "/Users/me/workspace/my-project";

describe("formatCommentsForTerminal", () => {
	it("should return empty string for no comments", () => {
		expect(formatCommentsForTerminal([])).toBe("");
	});

	it("should strip rootPath to produce relative paths", () => {
		const result = formatCommentsForTerminal(
			[
				makeComment({
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 42,
					content: "変数名を改善してください",
				}),
			],
			ROOT,
		);
		expect(result).toBe(
			"## Review Comments\nsrc/App.tsx:L42\n変数名を改善してください",
		);
	});

	it("should group comments by file and separate with =====", () => {
		const result = formatCommentsForTerminal(
			[
				makeComment({
					id: "c-1",
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 10,
					content: "comment A",
				}),
				makeComment({
					id: "c-2",
					filePath: `${ROOT}/src/hooks/useAuth.ts`,
					lineNumber: 5,
					content: "comment B",
				}),
				makeComment({
					id: "c-3",
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 20,
					content: "comment C",
				}),
			],
			ROOT,
		);
		expect(result).toBe(
			[
				"## Review Comments",
				"src/App.tsx:L10",
				"comment A",
				"=====",
				"src/App.tsx:L20",
				"comment C",
				"=====",
				"src/hooks/useAuth.ts:L5",
				"comment B",
			].join("\n"),
		);
	});

	it("should format range comment as L5-12", () => {
		const result = formatCommentsForTerminal(
			[
				makeComment({
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 5,
					content: "range comment",
					endLine: 12,
				}),
			],
			ROOT,
		);
		expect(result).toBe("## Review Comments\nsrc/App.tsx:L5-12\nrange comment");
	});

	it("should sort comments by line number within a file", () => {
		const result = formatCommentsForTerminal(
			[
				makeComment({
					id: "c-1",
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 50,
					content: "later",
				}),
				makeComment({
					id: "c-2",
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 10,
					content: "earlier",
				}),
			],
			ROOT,
		);
		expect(result).toBe(
			[
				"## Review Comments",
				"src/App.tsx:L10",
				"earlier",
				"=====",
				"src/App.tsx:L50",
				"later",
			].join("\n"),
		);
	});

	it("should not append separator after the last comment", () => {
		const result = formatCommentsForTerminal(
			[
				makeComment({
					filePath: `${ROOT}/src/App.tsx`,
					lineNumber: 1,
					content: "only one",
				}),
			],
			ROOT,
		);
		expect(result.endsWith("=====")).toBe(false);
	});

	it("should keep filePath as-is when rootPath is not provided", () => {
		const result = formatCommentsForTerminal([
			makeComment({
				filePath: "src/App.tsx",
				lineNumber: 1,
				content: "relative path comment",
			}),
		]);
		expect(result).toBe(
			"## Review Comments\nsrc/App.tsx:L1\nrelative path comment",
		);
	});
});
