import type * as Monaco from "monaco-editor";
import type { LineComment } from "@/types/comment";

export interface InlineCommentZone {
	afterLineNumber: number;
	zoneId: string;
	dispose: () => void;
}

export interface InlineCommentManager {
	update: (
		ranges: { start: number; end?: number }[],
		getComments: (line: number) => LineComment[],
	) => void;
	dispose: () => void;
}

const ITEM_HEIGHT = 24;
const PADDING = 8;

function createZoneDom(
	comments: LineComment[],
	editor: Monaco.editor.ICodeEditor,
): HTMLDivElement {
	const widget = document.createElement("div");
	widget.className = "comment-inline-widget";

	const { contentLeft, contentWidth } = editor.getLayoutInfo();
	widget.style.marginLeft = `${contentLeft}px`;
	widget.style.width = `${contentWidth}px`;

	for (const comment of comments) {
		const item = document.createElement("div");
		item.className = "comment-inline-item";

		const author = document.createElement("span");
		author.className = "comment-inline-author";
		author.textContent = comment.author.name;
		item.appendChild(author);

		if (comment.severity) {
			const severity = document.createElement("span");
			severity.className = `comment-inline-severity severity-${comment.severity}`;
			severity.textContent = comment.severity;
			item.appendChild(severity);
		}

		const content = document.createElement("span");
		content.className = "comment-inline-content";
		content.textContent = comment.content;
		item.appendChild(content);

		widget.appendChild(item);
	}

	return widget;
}

export function createInlineCommentManager(
	editor: Monaco.editor.ICodeEditor,
): InlineCommentManager {
	console.log("[inline-comment] manager created");
	let zones: InlineCommentZone[] = [];

	const layoutDisposable = editor.onDidLayoutChange((info) => {
		const domNodes = editor
			.getDomNode()
			?.querySelectorAll<HTMLDivElement>(".comment-inline-widget");
		if (!domNodes) return;
		for (const node of domNodes) {
			node.style.marginLeft = `${info.contentLeft}px`;
			node.style.width = `${info.contentWidth}px`;
		}
	});

	const removeAllZones = () => {
		if (zones.length === 0) return;
		editor.changeViewZones((accessor) => {
			for (const zone of zones) {
				accessor.removeZone(zone.zoneId);
			}
		});
		zones = [];
	};

	const update = (
		ranges: { start: number; end?: number }[],
		getComments: (line: number) => LineComment[],
	) => {
		console.log("[inline-comment] update called", { rangeCount: ranges.length });
		removeAllZones();

		const newZones: InlineCommentZone[] = [];

		editor.changeViewZones((accessor) => {
			for (const range of ranges) {
				const comments = getComments(range.start);
				if (comments.length === 0) continue;

				const domNode = createZoneDom(comments, editor);
				const afterLineNumber = range.end ?? range.start;
				const heightInPx = comments.length * ITEM_HEIGHT + PADDING;

				const zoneId = accessor.addZone({
					afterLineNumber,
					heightInPx,
					domNode,
					suppressMouseDown: true,
				});

				newZones.push({
					afterLineNumber,
					zoneId,
					dispose: () => {
						editor.changeViewZones((a) => {
							a.removeZone(zoneId);
						});
					},
				});
			}
		});

		zones = newZones;
	};

	const dispose = () => {
		removeAllZones();
		layoutDisposable.dispose();
	};

	return { update, dispose };
}
