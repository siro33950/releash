import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { createInlineCommentManager } from "./commentInlineWidget";

function makeComment(overrides: Partial<LineComment> = {}): LineComment {
	return {
		id: "c1",
		filePath: "test.ts",
		lineNumber: 10,
		content: "テストコメント",
		status: "unsent",
		createdAt: Date.now(),
		author: { type: "human", name: "User" },
		resolved: false,
		target: "local",
		...overrides,
	};
}

function createMockEditor() {
	const zones: {
		id: string;
		afterLineNumber: number;
		heightInPx: number;
		suppressMouseDown: boolean;
		domNode: HTMLElement;
	}[] = [];
	let nextId = 1;

	const accessor = {
		addZone: vi.fn(
			(zone: {
				afterLineNumber: number;
				heightInPx: number;
				domNode: HTMLElement;
				suppressMouseDown: boolean;
			}) => {
				const id = String(nextId++);
				zones.push({ id, ...zone });
				return id;
			},
		),
		removeZone: vi.fn(),
	};

	const layoutDisposable = { dispose: vi.fn() };

	const editor = {
		changeViewZones: vi.fn((cb: (a: typeof accessor) => void) => {
			cb(accessor);
		}),
		getLayoutInfo: vi.fn(() => ({ contentLeft: 48, contentWidth: 800 })),
		onDidLayoutChange: vi.fn(() => layoutDisposable),
		getDomNode: vi.fn(() => null),
	} as unknown as Parameters<typeof createInlineCommentManager>[0];

	return { editor, accessor, zones, layoutDisposable };
}

describe("createInlineCommentManager", () => {
	it("InlineCommentManager を返す", () => {
		const { editor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		expect(manager).toBeDefined();
		expect(typeof manager.update).toBe("function");
		expect(typeof manager.dispose).toBe("function");
	});

	it("update() で editor.changeViewZones が呼ばれる", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		const comments = [makeComment()];
		manager.update([{ start: 10 }], () => comments);

		expect(editor.changeViewZones).toHaveBeenCalled();
		expect(accessor.addZone).toHaveBeenCalled();
	});

	it("コメントが空の range では ViewZone が作成されない", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 10 }], () => []);

		expect(accessor.addZone).not.toHaveBeenCalled();
	});

	it("複数の range でコメントがある range のみ ViewZone が作成される", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		const comments = [makeComment({ lineNumber: 5 })];
		manager.update([{ start: 5 }, { start: 10 }, { start: 15 }], (line) =>
			line === 5 ? comments : [],
		);

		expect(accessor.addZone).toHaveBeenCalledTimes(1);
		expect(accessor.addZone).toHaveBeenCalledWith(
			expect.objectContaining({
				afterLineNumber: 5,
				suppressMouseDown: true,
			}),
		);
	});

	it("end が指定された range では afterLineNumber が end になる", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 5, end: 12 }], () => [makeComment()]);

		expect(accessor.addZone).toHaveBeenCalledWith(
			expect.objectContaining({
				afterLineNumber: 12,
			}),
		);
	});

	it("heightInPx がコメント数に基づいて計算される", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		const comments = [
			makeComment({ id: "c1" }),
			makeComment({ id: "c2" }),
			makeComment({ id: "c3" }),
		];
		manager.update([{ start: 10 }], () => comments);

		// 3 comments × 24px + 8px padding = 80px
		expect(accessor.addZone).toHaveBeenCalledWith(
			expect.objectContaining({
				heightInPx: 80,
			}),
		);
	});

	it("update() を再呼出しすると前回の zone が削除される", () => {
		const { editor, accessor } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 10 }], () => [makeComment()]);
		expect(accessor.addZone).toHaveBeenCalledTimes(1);

		manager.update([{ start: 20 }], () => [makeComment()]);
		expect(accessor.removeZone).toHaveBeenCalledWith("1");
		expect(accessor.addZone).toHaveBeenCalledTimes(2);
	});

	it("dispose() で全 zone がクリーンアップされる", () => {
		const { editor, accessor, layoutDisposable } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 10 }], () => [makeComment()]);
		manager.dispose();

		expect(accessor.removeZone).toHaveBeenCalledWith("1");
		expect(layoutDisposable.dispose).toHaveBeenCalled();
	});

	it("DOM 構造が正しく生成される", () => {
		const { editor, zones } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		const comments = [
			makeComment({
				author: { type: "human", name: "Alice" },
				severity: "warning",
				content: "修正が必要です",
			}),
		];
		manager.update([{ start: 10 }], () => comments);

		const dom = zones[0].domNode;
		expect(dom.className).toBe("comment-inline-widget");

		const item = dom.querySelector(".comment-inline-item");
		expect(item).not.toBeNull();

		const author = dom.querySelector(".comment-inline-author");
		expect(author?.textContent).toBe("Alice");

		const severity = dom.querySelector(".comment-inline-severity");
		expect(severity?.textContent).toBe("warning");
		expect(severity?.classList.contains("severity-warning")).toBe(true);

		const content = dom.querySelector(".comment-inline-content");
		expect(content?.textContent).toBe("修正が必要です");
	});

	it("severity がない場合は severity 要素が生成されない", () => {
		const { editor, zones } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 10 }], () => [
			makeComment({ severity: undefined }),
		]);

		const dom = zones[0].domNode;
		expect(dom.querySelector(".comment-inline-severity")).toBeNull();
	});

	it("contentLeft と contentWidth がスタイルに反映される", () => {
		const { editor, zones } = createMockEditor();
		const manager = createInlineCommentManager(editor);

		manager.update([{ start: 10 }], () => [makeComment()]);

		const dom = zones[0].domNode;
		expect(dom.style.marginLeft).toBe("48px");
		expect(dom.style.width).toBe("800px");
	});
});
