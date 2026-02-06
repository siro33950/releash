import type * as Monaco from "monaco-editor";
import type { LineComment } from "@/types/comment";

export interface MonacoContentWidget {
	getId(): string;
	getDomNode(): HTMLElement;
	getPosition(): Monaco.editor.IContentWidgetPosition | null;
}

export interface CreateCommentPeekOptions {
	lineNumber: number;
	endLine?: number;
	existingComments: LineComment[];
	onSubmit: (content: string) => void;
	onCancel: () => void;
}

export function createCommentPeekWidget(
	monaco: typeof Monaco,
	options: CreateCommentPeekOptions,
): MonacoContentWidget {
	const { lineNumber, endLine, existingComments, onSubmit, onCancel } = options;

	const domNode = document.createElement("div");
	domNode.className = "comment-peek-widget";

	// Header
	const header = document.createElement("div");
	header.className = "comment-peek-header";

	const title = document.createElement("span");
	title.className = "comment-peek-header-title";
	title.textContent = endLine != null
		? `Line ${lineNumber}-${endLine} - コメント`
		: `Line ${lineNumber} - コメント`;
	header.appendChild(title);

	const shortcutHint = document.createElement("span");
	shortcutHint.className = "comment-peek-shortcut-hint";
	shortcutHint.textContent = "⌘Enter で送信";
	header.appendChild(shortcutHint);

	const closeBtn = document.createElement("button");
	closeBtn.className = "comment-peek-close-btn";
	closeBtn.textContent = "\u00d7";
	closeBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		onCancel();
	});
	header.appendChild(closeBtn);

	domNode.appendChild(header);

	// Existing comments
	if (existingComments.length > 0) {
		const existing = document.createElement("div");
		existing.className = "comment-peek-existing";

		for (const comment of existingComments) {
			const item = document.createElement("div");
			item.className = "comment-peek-existing-item";

			const status = document.createElement("span");
			status.className = "comment-peek-status";
			status.textContent = comment.status;
			item.appendChild(status);

			const text = document.createElement("span");
			text.className = "comment-peek-comment-text";
			text.textContent = comment.content;
			item.appendChild(text);

			existing.appendChild(item);
		}

		domNode.appendChild(existing);
	}

	// Input area
	const inputArea = document.createElement("div");
	inputArea.className = "comment-peek-input-area";

	const textarea = document.createElement("textarea");
	textarea.className = "comment-peek-textarea";
	textarea.placeholder = "コメントを入力...";
	textarea.rows = 3;
	inputArea.appendChild(textarea);

	domNode.appendChild(inputArea);

	// Actions
	const actions = document.createElement("div");
	actions.className = "comment-peek-actions";

	const cancelBtn = document.createElement("button");
	cancelBtn.className = "comment-peek-cancel-btn";
	cancelBtn.textContent = "キャンセル";
	cancelBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		onCancel();
	});
	actions.appendChild(cancelBtn);

	const submitBtn = document.createElement("button");
	submitBtn.className = "comment-peek-submit-btn";
	submitBtn.textContent = "追加";
	submitBtn.addEventListener("click", (e) => {
		e.stopPropagation();
		const content = textarea.value.trim();
		if (content) {
			onSubmit(content);
		} else {
			onCancel();
		}
	});
	actions.appendChild(submitBtn);

	domNode.appendChild(actions);

	// Key events on the whole widget
	domNode.addEventListener("keydown", (e) => {
		e.stopPropagation();

		if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
			e.preventDefault();
			const content = textarea.value.trim();
			if (content) {
				onSubmit(content);
			} else {
				onCancel();
			}
		} else if (e.key === "Escape") {
			e.preventDefault();
			onCancel();
		}
	});

	setTimeout(() => textarea.focus(), 0);

	return {
		getId: () => "comment-input-widget",
		getDomNode: () => domNode,
		getPosition: () => ({
			position: { lineNumber: (endLine ?? lineNumber) + 1, column: 1 },
			preference: [
				monaco.editor.ContentWidgetPositionPreference.ABOVE,
			],
		}),
	};
}
