import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import type { Thread, ThreadEntry } from "@/types/thread";
import type {
	CommentThreadOptions,
	CommentThreadZone,
} from "./commentThreadWidget";

function makeEntry(overrides: Partial<ThreadEntry> = {}): ThreadEntry {
	return {
		id: "e-1",
		content: "Test comment",
		isAi: false,
		createdAt: Date.now(),
		...overrides,
	};
}

function makeThread(overrides: Partial<Thread> = {}): Thread {
	return {
		id: "t-1",
		filePath: "test.ts",
		lineNumber: 10,
		entries: [makeEntry()],
		resolved: false,
		createdAt: Date.now(),
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
		opts: Partial<CommentThreadOptions> & { thread: Thread },
	): Promise<CommentThreadZone> {
		const { createCommentThread } = await import("./commentThreadWidget");
		let zone: CommentThreadZone | undefined;
		act(() => {
			zone = createCommentThread(editor as never, {
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
		const zone = await createZone(editor, {
			thread: makeThread({ lineNumber: 42 }),
		});

		const header = zone.domNode.querySelector(".comment-thread-header-title");
		expect(header?.textContent).toBe("L42");
		disposeZone(zone);
	});

	it("should show line range in header for multi-line comment", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread({ lineNumber: 42, endLine: 48 }),
		});

		const header = zone.domNode.querySelector(".comment-thread-header-title");
		expect(header?.textContent).toBe("L42-48");
		disposeZone(zone);
	});

	it("should render existing comments with content", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread({
				entries: [
					makeEntry({ id: "e-1", content: "First comment" }),
					makeEntry({ id: "e-2", content: "Second comment" }),
				],
			}),
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
			thread: makeThread({ severity: "error" }),
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
			thread: makeThread({ entries: [] }),
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
			thread: makeThread({ entries: [] }),
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
			thread: makeThread({ entries: [] }),
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

	it("should call onDeleteThread when delete button is clicked", async () => {
		const editor = makeMockEditor();
		const onDelete = vi.fn();
		const zone = await createZone(editor, {
			thread: makeThread({ id: "t-del-1" }),
			onDeleteThread: onDelete,
		});

		const deleteBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-action-delete",
		);
		act(() => {
			deleteBtn?.click();
		});

		expect(onDelete).toHaveBeenCalledWith("t-del-1");
		disposeZone(zone);
	});

	it("should call onResolveThread when resolve button is clicked", async () => {
		const editor = makeMockEditor();
		const onResolve = vi.fn();
		const zone = await createZone(editor, {
			thread: makeThread({ id: "t-res-1" }),
			onResolveThread: onResolve,
		});

		const resolveBtn = zone.domNode.querySelector<HTMLButtonElement>(
			".comment-thread-action-resolve",
		);
		act(() => {
			resolveBtn?.click();
		});

		expect(onResolve).toHaveBeenCalledWith("t-res-1");
		disposeZone(zone);
	});

	it("should have a textarea for reply input", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread({ entries: [] }),
		});

		const textarea = zone.domNode.querySelector(".comment-thread-textarea");
		expect(textarea).not.toBeNull();
		expect((textarea as HTMLTextAreaElement).placeholder).toBe(
			"Type a reply...",
		);
		disposeZone(zone);
	});

	it("should create a ViewZone and OverlayWidget", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread(),
		});

		expect(editor.changeViewZones).toHaveBeenCalled();
		expect(editor.addOverlayWidget).toHaveBeenCalled();
		expect(zone.zoneId).toBe("zone-1");
		disposeZone(zone);
	});

	it("should set width excluding minimap and scrollbar (VSCode ZoneWidget formula)", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread(),
		});

		// width:800 - minimapWidth:0 - scrollbarWidth:14 - contentLeft:60 = 726px
		expect(zone.domNode.style.width).toBe("726px");
		expect(zone.domNode.style.left).toBe("60px");
		disposeZone(zone);
	});

	it("should remove OverlayWidget on dispose", async () => {
		const editor = makeMockEditor();
		const zone = await createZone(editor, {
			thread: makeThread(),
		});

		disposeZone(zone);
		expect(editor.removeOverlayWidget).toHaveBeenCalled();
	});
});
