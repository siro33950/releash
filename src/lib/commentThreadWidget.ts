import type * as Monaco from "monaco-editor";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { CommentThread } from "@/components/panels/CommentThread";
import type { Thread } from "@/types/thread";

export interface CommentThreadOptions {
	thread: Thread;
	onSubmit: (content: string) => void;
	onCancel: () => void;
	onDeleteThread?: (threadId: string) => void;
	onUpdateEntry?: (threadId: string, entryId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
	onResolveThread?: (threadId: string) => void;
}

export interface CommentThreadZone {
	zoneId: string;
	domNode: HTMLElement;
	dispose: () => void;
	update: (partial: Partial<Pick<CommentThreadOptions, "thread">>) => void;
}

const HEADER_HEIGHT = 32;
const ITEM_HEIGHT = 52;
const ITEM_MAX_VISIBLE = 4;
const INPUT_AREA_HEIGHT = 88;
const ACTIONS_HEIGHT = 40;
const PADDING = 8;

function computeInitialHeight(entryCount: number): number {
	const itemsHeight =
		entryCount > 0
			? Math.min(entryCount * ITEM_HEIGHT, ITEM_MAX_VISIBLE * ITEM_HEIGHT)
			: 0;
	return (
		HEADER_HEIGHT + itemsHeight + INPUT_AREA_HEIGHT + ACTIONS_HEIGHT + PADDING
	);
}

// VSCode ZoneWidget 方式: editor全体幅 - minimap幅 - スクロールバー幅
function computeWidgetWidth(info: Monaco.editor.EditorLayoutInfo): number {
	return info.width - info.minimap.minimapWidth - info.verticalScrollbarWidth;
}

export function createCommentThread(
	editor: Monaco.editor.ICodeEditor,
	options: CommentThreadOptions,
): CommentThreadZone {
	const {
		thread,
		onSubmit,
		onCancel,
		onDeleteThread,
		onUpdateEntry,
		onCopyThread,
		onResolveThread,
	} = options;

	let currentThread = thread;

	// VSCode ZoneWidget 方式:
	// ViewZone = 空のdomNodeでスペース確保のみ
	// OverlayWidget = 実際のウィジェット表示（幅・位置を自前制御）
	const zoneDomNode = document.createElement("div");
	zoneDomNode.style.overflow = "hidden";

	const widgetNode = document.createElement("div");
	widgetNode.className = "comment-thread-widget";
	widgetNode.style.position = "absolute";

	// 幅・左端の初期設定
	const applyLayout = (info: Monaco.editor.EditorLayoutInfo) => {
		widgetNode.style.left = `${info.contentLeft}px`;
		widgetNode.style.width = `${computeWidgetWidth(info) - info.contentLeft}px`;
	};

	const layout = editor.getLayoutInfo();
	applyLayout(layout);

	const layoutDisposable = editor.onDidLayoutChange(applyLayout);

	// Prevent editor from stealing focus
	widgetNode.addEventListener("mousedown", (e) => {
		e.stopPropagation();
	});

	// Prevent editor from stealing scroll events when widget content is scrollable
	widgetNode.addEventListener(
		"wheel",
		(e) => {
			const scrollable = widgetNode.querySelector<HTMLElement>(
				".comment-thread-items",
			);
			if (!scrollable) return;

			const { scrollTop, scrollHeight, clientHeight } = scrollable;
			const atTop = scrollTop <= 0 && e.deltaY < 0;
			const atBottom = scrollTop + clientHeight >= scrollHeight && e.deltaY > 0;

			// Only propagate to editor if list is not scrollable or at boundary
			if (scrollHeight > clientHeight && !atTop && !atBottom) {
				e.stopPropagation();
			}
		},
		{ passive: true },
	);

	// Keyboard shortcuts on the whole widget
	widgetNode.addEventListener("keydown", (e) => {
		e.stopPropagation();
		if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
			e.preventDefault();
			const textarea = widgetNode.querySelector<HTMLTextAreaElement>(
				".comment-thread-textarea",
			);
			const content = textarea?.value.trim();
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

	// React レンダリング
	const root = createRoot(widgetNode);
	const renderWidget = () => {
		root.render(
			createElement(CommentThread, {
				thread: currentThread,
				onSubmit,
				onCancel,
				onDeleteThread,
				onUpdateEntry,
				onCopyThread,
				onResolveThread,
			}),
		);
	};
	renderWidget();

	// ViewZone（スペース確保 + OverlayWidget 位置同期）
	// heightInPx は初期推定値。React描画後に ResizeObserver で実際の高さに同期する。
	const afterLineNumber = thread.endLine ?? thread.lineNumber;
	const zoneConfig: Monaco.editor.IViewZone = {
		afterLineNumber,
		heightInPx: computeInitialHeight(thread.entries.length),
		domNode: zoneDomNode,
		suppressMouseDown: true,
		onDomNodeTop: (top: number) => {
			widgetNode.style.top = `${top}px`;
		},
		onComputedHeight: () => {
			// widgetNode の高さは固定しない（コンテンツの自然な高さで表示）
		},
	};

	let zoneId = "";
	editor.changeViewZones((accessor) => {
		zoneId = accessor.addZone(zoneConfig);
	});

	// OverlayWidget 登録
	const overlayWidget: Monaco.editor.IOverlayWidget = {
		getId: () => `comment-thread-${zoneId}`,
		getDomNode: () => widgetNode,
		getPosition: () => null,
	};
	editor.addOverlayWidget(overlayWidget);

	// React描画後に実際の高さを計測し、ViewZoneの高さを同期する
	const resizeObserver = new ResizeObserver(() => {
		const actualHeight = widgetNode.offsetHeight;
		if (actualHeight > 0 && actualHeight !== zoneConfig.heightInPx) {
			zoneConfig.heightInPx = actualHeight;
			editor.changeViewZones((accessor) => {
				accessor.layoutZone(zoneId);
			});
		}
	});
	resizeObserver.observe(widgetNode);

	const dispose = () => {
		resizeObserver.disconnect();
		root.unmount();
		layoutDisposable.dispose();
		editor.removeOverlayWidget(overlayWidget);
		editor.changeViewZones((accessor) => {
			accessor.removeZone(zoneId);
		});
	};

	const update = (partial: Partial<Pick<CommentThreadOptions, "thread">>) => {
		if (partial.thread !== undefined) {
			currentThread = partial.thread;
			const newAfterLine = partial.thread.endLine ?? partial.thread.lineNumber;
			if (zoneConfig.afterLineNumber !== newAfterLine) {
				zoneConfig.afterLineNumber = newAfterLine;
				editor.changeViewZones((accessor) => {
					accessor.layoutZone(zoneId);
				});
			}
		}
		renderWidget();
	};

	return {
		zoneId,
		domNode: widgetNode,
		dispose,
		update,
	};
}
