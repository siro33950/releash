import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import type {
	CommentThreadOptions,
	CommentThreadZone,
} from "./commentThreadWidget";

function makeComment(overrides: Partial<LineComment> = {}): LineComment {
	return {
		id: "test-id",
		filePath: "test.ts",
		lineNumber: 10,
		content: "Test comment",
		status: "unsent",
		createdAt: Date.now(),
		resolved: false,
		target: "local",
		...overrides,
	};
}

describe("createCommentThread", () => {
	function makeMockEditor() {
		let zoneIdCounter = 0;

		return {
			getLayoutInfo: () => ({
				width: 800,
				contentLeft: 60,
				contentWidth: 600,
				minimap: { minimapWidth: 0 },
				verticalScrollbarWidth: 14,
				height: 400,
			}),
			onDidLayoutChange: vi.fn(() => ({ dispose: vi.fn() })),
			changeViewZones: vi.fn(
				(
					cb: (accessor: {
						addZone: (zone: {
							afterLineNumber: number;
							heightInPx: number;
							domNode: HTMLElement;
						}) => string;
						removeZone: (id: string) => void;
					}) => void,
				) => {
					cb({
						addZone: () => {
							zoneIdCounter++;
							return `zone-${zoneIdCounter}`;
						},
						removeZone: vi.fn(),
					});
				},
			),
			addOverlayWidget: vi.fn(),
			removeOverlayWidget: vi.fn(),
		};
	}

	async function createZone(
		editor: ReturnType<typeof makeMockEditor>,
		opts: Partial<CommentThreadOptions> & { lineNumber: number },
	): Promise<CommentThreadZone> {
		const { createCommentThread } = await import("./commentThreadWidget");
		let zone: CommentThreadZone | undefined;
		act(() => {
			zone = createCommentThread(editor as never, {
				comments: [],
				onSubmit: vi.fn(),
				onCancel: vi.fn(),
				...opts,
			});
		});
		return zone as CommentThreadZone;
	}

	function disposeZone(zone: CommentThreadZone) {
		act(() => zone.dispose());
	}

	it("should create a widget with header showing line number", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, { lineNumber: 42 });

		const header = zone.domNode.querySelector(".comment-thread-header-title");
		expect(header?.textContent).toBe("L42");
		disposeZone(zone);
	});

	it("should show line range in header for multi-line comment", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			lineNumber: 42,
			endLine: 48,
		});

		const header = zone.domNode.querySelector(".comment-thread-header-title");
		expect(header?.textContent).toBe("L42-48");
		disposeZone(zone);
	});

	it("should render existing comments with content", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			lineNumber: 10,
			comments: [
				makeComment({ content: "First comment" }),
				makeComment({ id: "id2", content: "Second comment" }),
			],
		});

		const items = zone.domNode.querySelectorAll(".comment-thread-item");
		expect(items.length).toBe(2);
		expect(
			items[0].querySelector(".comment-thread-item-content")?.textContent,
		).toBe("First comment");
		expect(
			items[1].querySelector(".comment-thread-item-content")?.textContent,
		).toBe("Second comment");
		disposeZone(zone);
	});

	it("should show severity badge when present", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			lineNumber: 10,
			comments: [makeComment({ severity: "error" })],
		});

		const badge = zone.domNode.querySelector(".comment-thread-severity");
		expect(badge).not.toBeNull();
		expect(badge?.textContent).toBe("error");
		expect(badge?.classList.contains("severity-error")).toBe(true);
		disposeZone(zone);
	});

	it("should call onSubmit when submit button is clicked", async () => {
		const editor = makeMockEditor();
		const onSubmit = vi.fn();
		const zone = await createZone(editor, {
			lineNumber: 10,
			onSubmit,
		});

		const textarea = zone.domNode.querySelector<HTMLTextAreaElement>(
			".comment-thread-textarea",
		);
		expect(textarea).not.toBeNull();
		act(() => {
			if (textarea) {
				const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
					HTMLTextAreaElement.prototype,
					"value",
				)?.set;
				nativeInputValueSetter?.call(textarea, "New comment");
				textarea.dispatchEvent(new Event("input", { bubbles: true }));
			}
		});

		const submitBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-submit-btn",
		);
		act(() => {
			submitBtn?.click();
		});

		expect(onSubmit).toHaveBeenCalledWith("New comment");
		disposeZone(zone);
	});

	it("should call onCancel when cancel button is clicked", async () => {
		const editor = makeMockEditor();
		const onCancel = vi.fn();
		const zone = await createZone(editor, {
			lineNumber: 10,
			onCancel,
		});

		const cancelBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-cancel-btn",
		);
		act(() => {
			cancelBtn?.click();
		});

		expect(onCancel).toHaveBeenCalled();
		disposeZone(zone);
	});

	it("should call onCancel when close button is clicked", async () => {
		const editor = makeMockEditor();
		const onCancel = vi.fn();
		const zone = await createZone(editor, {
			lineNumber: 10,
			onCancel,
		});

		const closeBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-close-btn",
		);
		act(() => {
			closeBtn?.click();
		});

		expect(onCancel).toHaveBeenCalled();
		disposeZone(zone);
	});

	it("should call onDeleteComment when delete button is clicked", async () => {
		const editor = makeMockEditor();
		const onDelete = vi.fn();
		const zone = await createZone(editor, {
			lineNumber: 10,
			comments: [makeComment({ id: "del-1" })],
			onDeleteComment: onDelete,
		});

		const deleteBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-action-delete",
		);
		act(() => {
			deleteBtn?.click();
		});

		expect(onDelete).toHaveBeenCalledWith("del-1");
		disposeZone(zone);
	});

	it("should filter resolved comments when showResolvedComments is false", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			lineNumber: 10,
			comments: [
				makeComment({ id: "active", resolved: false }),
				makeComment({ id: "resolved", resolved: true }),
			],
			showResolvedComments: false,
		});

		const items = zone.domNode.querySelectorAll(".comment-thread-item");
		expect(items.length).toBe(1);
		disposeZone(zone);
	});

	it("should have a textarea for reply input", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, { lineNumber: 10 });

		const textarea = zone.domNode.querySelector(".comment-thread-textarea");
		expect(textarea).not.toBeNull();
		expect((textarea as HTMLTextAreaElement).placeholder).toBe("返信を入力...");
		disposeZone(zone);
	});

	it("should create a ViewZone and OverlayWidget", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, { lineNumber: 10 });

		expect(editor.changeViewZones).toHaveBeenCalled();
		expect(editor.addOverlayWidget).toHaveBeenCalled();
		expect(zone.zoneId).toBe("zone-1");
		disposeZone(zone);
	});

	it("should set width excluding minimap and scrollbar (VSCode ZoneWidget formula)", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, { lineNumber: 10 });

		// width:800 - minimapWidth:0 - scrollbarWidth:14 - contentLeft:60 = 726px
		expect(zone.domNode.style.width).toBe("726px");
		expect(zone.domNode.style.left).toBe("60px");
		disposeZone(zone);
	});

	it("should remove OverlayWidget on dispose", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, { lineNumber: 10 });

		disposeZone(zone);
		expect(editor.removeOverlayWidget).toHaveBeenCalled();
	});
});
