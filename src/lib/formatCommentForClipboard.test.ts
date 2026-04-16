import { describe, expect, it } from "vitest";
import type { Thread } from "@/types/thread";
import { formatCommentForClipboard } from "./formatCommentForClipboard";

function makeThread(
	overrides: Partial<Thread> & {
		filePath: string;
		lineNumber: number;
		content: string;
	},
): Thread {
	const { content, ...rest } = overrides;
	return {
		id: "t-1",
		entries: [
			{
				id: "e-1",
				content,
				createdAt: Date.now(),
			},
		],
		createdAt: Date.now(),
		resolved: false,
		...rest,
	};
}

describe("formatCommentForClipboard", () => {
	it("should format a single-line comment", () => {
		const result = formatCommentForClipboard(
			makeThread({
				filePath: "/src/App.tsx",
				lineNumber: 42,
				content: "変数名を改善してください",
			}),
		);
		expect(result).toBe("/src/App.tsx:L42\n変数名を改善してください");
	});

	it("should format a range comment with endLine", () => {
		const result = formatCommentForClipboard(
			makeThread({
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
