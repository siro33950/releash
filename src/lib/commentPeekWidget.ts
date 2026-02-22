import type * as Monaco from "monaco-editor";
import type { LineComment } from "@/types/comment";

export interface CommentViewZone {
	domNode: HTMLElement;
	zoneId: string;
	dispose: () => void;
}

export interface CreateCommentPeekOptions {
	lineNumber: number;
	endLine?: number;
	existingComments: LineComment[];
	showSentComments?: boolean;
	onSubmit: (content: string) => void;
	onCancel: () => void;
}

const HEADER_HEIGHT = 32;
const EXISTING_MAX_HEIGHT = 112;
const INPUT_AREA_HEIGHT = 88;
const ACTIONS_HEIGHT = 40;
const PADDING = 8;

function computeZoneHeight(existingCount: number): number {
	const existingHeight =
		existingCount > 0 ? Math.min(existingCount * 28, EXISTING_MAX_HEIGHT) : 0;
	return (
		HEADER_HEIGHT +
		existingHeight +
		INPUT_AREA_HEIGHT +
		ACTIONS_HEIGHT +
		PADDING
	);
}

export function openCommentViewZone(
	editor: Monaco.editor.ICodeEditor,
	options: CreateCommentPeekOptions,
): CommentViewZone {
	const {
		lineNumber,
		endLine,
		existingComments: rawExistingComments,
		showSentComments = true,
		onSubmit,
		onCancel,
	} = options;

	const existingComments = showSentComments
		? rawExistingComments
		: rawExistingComments.filter((c) => c.status !== "sent");

	const domNode = document.createElement("div");
	domNode.className = "comment-peek-widget";

	const { contentLeft, contentWidth } = editor.getLayoutInfo();
	domNode.style.marginLeft = `${contentLeft}px`;
	domNode.style.width = `${Math.min(420, contentWidth)}px`;

	// Header
	const header = document.createElement("div");
	header.className = "comment-peek-header";

	const title = document.createElement("span");
	title.className = "comment-peek-header-title";
	title.textContent =
		endLine != null
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

	domNode.addEventListener("mousedown", (e) => {
		e.stopPropagation();
	});

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

	// ViewZone
	const afterLineNumber = endLine ?? lineNumber;
	const heightInPx = computeZoneHeight(existingComments.length);

	let zoneId = "";
	editor.changeViewZones((accessor) => {
		zoneId = accessor.addZone({
			afterLineNumber,
			heightInPx,
			domNode,
			suppressMouseDown: true,
		});
	});

	const dispose = () => {
		editor.changeViewZones((accessor) => {
			accessor.removeZone(zoneId);
		});
	};

	return {
		domNode,
		zoneId,
		dispose,
	};
}
