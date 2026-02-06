import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { createCommentPeekWidget } from "./commentPeekWidget";

const mockMonaco = {
	editor: {
		ContentWidgetPositionPreference: { ABOVE: 1 },
	},
} as Parameters<typeof createCommentPeekWidget>[0];

function makeComment(overrides: Partial<LineComment> = {}): LineComment {
	return {
		id: "c1",
		filePath: "test.ts",
		lineNumber: 10,
		content: "既存コメント",
		status: "unsent",
		createdAt: Date.now(),
		...overrides,
	};
}

describe("createCommentPeekWidget", () => {
	it("DOM構造が正しく生成される", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		expect(dom.className).toBe("comment-peek-widget");

		const header = dom.querySelector(".comment-peek-header");
		expect(header).not.toBeNull();

		const title = dom.querySelector(".comment-peek-header-title");
		expect(title?.textContent).toBe("Line 10 - コメント");

		const closeBtn = dom.querySelector(".comment-peek-close-btn");
		expect(closeBtn?.textContent).toBe("\u00d7");

		const textarea = dom.querySelector(
			".comment-peek-textarea",
		) as HTMLTextAreaElement;
		expect(textarea).not.toBeNull();
		expect(textarea.placeholder).toBe("コメントを入力...");

		const cancelBtn = dom.querySelector(".comment-peek-cancel-btn");
		expect(cancelBtn?.textContent).toBe("キャンセル");

		const submitBtn = dom.querySelector(".comment-peek-submit-btn");
		expect(submitBtn?.textContent).toBe("追加");
	});

	it("既存コメントがない場合、existing セクションが生成されない", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 5,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		expect(dom.querySelector(".comment-peek-existing")).toBeNull();
	});

	it("既存コメントがある場合、正しく表示される", () => {
		const comments = [
			makeComment({ id: "c1", content: "最初のコメント", status: "sent" }),
			makeComment({ id: "c2", content: "2番目", status: "unsent" }),
		];

		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: comments,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const existing = dom.querySelector(".comment-peek-existing");
		expect(existing).not.toBeNull();

		const items = existing!.querySelectorAll(".comment-peek-existing-item");
		expect(items).toHaveLength(2);

		expect(items[0].querySelector(".comment-peek-status")?.textContent).toBe(
			"sent",
		);
		expect(
			items[0].querySelector(".comment-peek-comment-text")?.textContent,
		).toBe("最初のコメント");
		expect(items[1].querySelector(".comment-peek-status")?.textContent).toBe(
			"unsent",
		);
		expect(
			items[1].querySelector(".comment-peek-comment-text")?.textContent,
		).toBe("2番目");
	});

	it("追加ボタンクリックで onSubmit が呼ばれる", () => {
		const onSubmit = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const textarea = dom.querySelector(
			".comment-peek-textarea",
		) as HTMLTextAreaElement;
		textarea.value = "新しいコメント";

		const submitBtn = dom.querySelector(
			".comment-peek-submit-btn",
		) as HTMLButtonElement;
		submitBtn.click();

		expect(onSubmit).toHaveBeenCalledWith("新しいコメント");
	});

	it("空入力で追加ボタンクリックすると onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const dom = widget.getDomNode();
		const submitBtn = dom.querySelector(
			".comment-peek-submit-btn",
		) as HTMLButtonElement;
		submitBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("キャンセルボタンクリックで onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const dom = widget.getDomNode();
		const cancelBtn = dom.querySelector(
			".comment-peek-cancel-btn",
		) as HTMLButtonElement;
		cancelBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("閉じるボタンクリックで onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const dom = widget.getDomNode();
		const closeBtn = dom.querySelector(
			".comment-peek-close-btn",
		) as HTMLButtonElement;
		closeBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("Cmd+Enter で onSubmit が呼ばれる", () => {
		const onSubmit = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const textarea = dom.querySelector(
			".comment-peek-textarea",
		) as HTMLTextAreaElement;
		textarea.value = "Cmd+Enter コメント";

		dom.dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "Enter",
				metaKey: true,
				bubbles: true,
			}),
		);

		expect(onSubmit).toHaveBeenCalledWith("Cmd+Enter コメント");
	});

	it("Ctrl+Enter で onSubmit が呼ばれる", () => {
		const onSubmit = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const textarea = dom.querySelector(
			".comment-peek-textarea",
		) as HTMLTextAreaElement;
		textarea.value = "Ctrl+Enter コメント";

		dom.dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "Enter",
				ctrlKey: true,
				bubbles: true,
			}),
		);

		expect(onSubmit).toHaveBeenCalledWith("Ctrl+Enter コメント");
	});

	it("Escape で onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const dom = widget.getDomNode();
		dom.dispatchEvent(
			new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);

		expect(onCancel).toHaveBeenCalled();
	});

	it("plain Enter はコールバックを呼ばない（改行用）", () => {
		const onSubmit = vi.fn();
		const onCancel = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel,
		});

		const dom = widget.getDomNode();
		dom.dispatchEvent(
			new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
		);

		expect(onSubmit).not.toHaveBeenCalled();
		expect(onCancel).not.toHaveBeenCalled();
	});

	it("Cmd+Enter で空入力の場合 onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const onSubmit = vi.fn();
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel,
		});

		const dom = widget.getDomNode();
		dom.dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "Enter",
				metaKey: true,
				bubbles: true,
			}),
		);

		expect(onSubmit).not.toHaveBeenCalled();
		expect(onCancel).toHaveBeenCalled();
	});

	it("getId が正しい値を返す", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(widget.getId()).toBe("comment-input-widget");
	});

	it("範囲コメントのタイトルが Line X-Y 形式になる", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 5,
			endLine: 12,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const title = dom.querySelector(".comment-peek-header-title");
		expect(title?.textContent).toBe("Line 5-12 - コメント");
	});

	it("範囲コメントの getPosition が endLine + 1 を返す", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 5,
			endLine: 12,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const pos = widget.getPosition();
		expect(pos?.position?.lineNumber).toBe(13);
	});

	it("ヘッダーにショートカットヒントが表示される", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = widget.getDomNode();
		const hint = dom.querySelector(".comment-peek-shortcut-hint");
		expect(hint).not.toBeNull();
		expect(hint?.textContent).toBe("⌘Enter で送信");
	});

	it("getPosition が lineNumber + 1 の ABOVE を返す", () => {
		const widget = createCommentPeekWidget(mockMonaco, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const pos = widget.getPosition();
		expect(pos).toEqual({
			position: { lineNumber: 11, column: 1 },
			preference: [1],
		});
	});
});
