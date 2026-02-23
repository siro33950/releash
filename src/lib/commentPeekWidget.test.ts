import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { openCommentViewZone } from "./commentPeekWidget";

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
	} as unknown as Parameters<typeof openCommentViewZone>[0];

	return { editor, accessor, zones, layoutDisposable };
}

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

describe("openCommentViewZone", () => {
	it("DOM構造が正しく生成される", () => {
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
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
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 5,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(zone.domNode.querySelector(".comment-peek-existing")).toBeNull();
	});

	it("既存コメントがある場合、正しく表示される", () => {
		const comments = [
			makeComment({ id: "c1", content: "最初のコメント", status: "sent" }),
			makeComment({ id: "c2", content: "2番目", status: "unsent" }),
		];

		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: comments,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
		const existing = dom.querySelector(".comment-peek-existing");
		expect(existing).not.toBeNull();

		const items = existing
			? existing.querySelectorAll(".comment-peek-existing-item")
			: [];
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
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
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
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const submitBtn = zone.domNode.querySelector(
			".comment-peek-submit-btn",
		) as HTMLButtonElement;
		submitBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("キャンセルボタンクリックで onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const cancelBtn = zone.domNode.querySelector(
			".comment-peek-cancel-btn",
		) as HTMLButtonElement;
		cancelBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("閉じるボタンクリックで onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		const closeBtn = zone.domNode.querySelector(
			".comment-peek-close-btn",
		) as HTMLButtonElement;
		closeBtn.click();

		expect(onCancel).toHaveBeenCalled();
	});

	it("Cmd+Enter で onSubmit が呼ばれる", () => {
		const onSubmit = vi.fn();
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
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
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
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
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel,
		});

		zone.domNode.dispatchEvent(
			new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		);

		expect(onCancel).toHaveBeenCalled();
	});

	it("plain Enter はコールバックを呼ばない（改行用）", () => {
		const onSubmit = vi.fn();
		const onCancel = vi.fn();
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel,
		});

		zone.domNode.dispatchEvent(
			new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
		);

		expect(onSubmit).not.toHaveBeenCalled();
		expect(onCancel).not.toHaveBeenCalled();
	});

	it("Cmd+Enter で空入力の場合 onCancel が呼ばれる", () => {
		const onCancel = vi.fn();
		const onSubmit = vi.fn();
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit,
			onCancel,
		});

		zone.domNode.dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "Enter",
				metaKey: true,
				bubbles: true,
			}),
		);

		expect(onSubmit).not.toHaveBeenCalled();
		expect(onCancel).toHaveBeenCalled();
	});

	it("ViewZone が afterLineNumber = lineNumber で追加される", () => {
		const { editor, zones } = createMockEditor();
		openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(zones).toHaveLength(1);
		expect(zones[0].afterLineNumber).toBe(10);
		expect(zones[0].suppressMouseDown).toBe(true);
	});

	it("範囲コメントのタイトルが Line X-Y 形式になる", () => {
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 5,
			endLine: 12,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const title = zone.domNode.querySelector(".comment-peek-header-title");
		expect(title?.textContent).toBe("Line 5-12 - コメント");
	});

	it("範囲コメントの afterLineNumber が endLine になる", () => {
		const { editor, zones } = createMockEditor();
		openCommentViewZone(editor, {
			lineNumber: 5,
			endLine: 12,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(zones[0].afterLineNumber).toBe(12);
	});

	it("ヘッダーにショートカットヒントが表示される", () => {
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const hint = zone.domNode.querySelector(".comment-peek-shortcut-hint");
		expect(hint).not.toBeNull();
		expect(hint?.textContent).toBe("⌘Enter で送信");
	});

	it("dispose が removeZone を呼ぶ", () => {
		const { editor, accessor, zones } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const zoneId = zones[0].id;
		zone.dispose();

		expect(accessor.removeZone).toHaveBeenCalledWith(zoneId);
	});

	it("showSentComments=false で sent コメントが非表示になる", () => {
		const comments = [
			makeComment({ id: "c1", content: "sent comment", status: "sent" }),
			makeComment({ id: "c2", content: "unsent comment", status: "unsent" }),
		];

		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: comments,
			showSentComments: false,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const dom = zone.domNode;
		const existing = dom.querySelector(".comment-peek-existing");
		expect(existing).not.toBeNull();

		const items = existing
			? existing.querySelectorAll(".comment-peek-existing-item")
			: [];
		expect(items).toHaveLength(1);
		expect(
			items[0].querySelector(".comment-peek-comment-text")?.textContent,
		).toBe("unsent comment");
	});

	it("showSentComments=true で全コメントが表示される", () => {
		const comments = [
			makeComment({ id: "c1", content: "sent comment", status: "sent" }),
			makeComment({ id: "c2", content: "unsent comment", status: "unsent" }),
		];

		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: comments,
			showSentComments: true,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const items = zone.domNode.querySelectorAll(".comment-peek-existing-item");
		expect(items).toHaveLength(2);
	});

	it("showSentComments 未指定時はデフォルトで全コメント表示", () => {
		const comments = [
			makeComment({ id: "c1", content: "sent", status: "sent" }),
			makeComment({ id: "c2", content: "unsent", status: "unsent" }),
		];

		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: comments,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const items = zone.domNode.querySelectorAll(".comment-peek-existing-item");
		expect(items).toHaveLength(2);
	});

	it("domNode に contentLeft の marginLeft が設定される", () => {
		const { editor } = createMockEditor();
		const zone = openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(zone.domNode.style.marginLeft).toBe("48px");
	});

	it("既存コメントの数に応じて heightInPx が変わる", () => {
		const comments = [
			makeComment({ id: "c1", content: "comment1" }),
			makeComment({ id: "c2", content: "comment2" }),
		];

		const { editor, zones: zonesWithComments } = createMockEditor();
		openCommentViewZone(editor, {
			lineNumber: 10,
			existingComments: comments,
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		const { editor: editor2, zones: zonesWithout } = createMockEditor();
		openCommentViewZone(editor2, {
			lineNumber: 10,
			existingComments: [],
			onSubmit: vi.fn(),
			onCancel: vi.fn(),
		});

		expect(zonesWithComments[0].heightInPx).toBeGreaterThan(
			zonesWithout[0].heightInPx,
		);
	});
});
