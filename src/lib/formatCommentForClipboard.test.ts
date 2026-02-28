import { describe, expect, it } from "vitest";
import type { LineComment } from "@/types/comment";
import { formatCommentForClipboard } from "./formatCommentForClipboard";

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
		author: { type: "human", name: "User" },
		resolved: false,
		target: "local",
		...overrides,
	};
}

describe("formatCommentForClipboard", () => {
	it("should format a single-line comment", () => {
		const result = formatCommentForClipboard(
			makeComment({
				filePath: "/src/App.tsx",
				lineNumber: 42,
				content: "変数名を改善してください",
			}),
		);
		expect(result).toBe("/src/App.tsx:L42\n変数名を改善してください");
	});

	it("should format a range comment with endLine", () => {
		const result = formatCommentForClipboard(
			makeComment({
				filePath: "/src/hooks/useAuth.ts",
				lineNumber: 5,
				content: "この範囲をリファクタしてください",
				endLine: 12,
			}),
		);
		expect(result).toBe(
			"/src/hooks/useAuth.ts:L5-12\nこの範囲をリファクタしてください",
		);
	});
});
