import { lazy, Suspense } from "react";
import type { ChangeGroup } from "@/lib/computeHunks";
import type { DiffMode, Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";
import { ImageDiffViewer } from "./ImageDiffViewer";
import { MarkdownDiffViewer } from "./MarkdownDiffViewer";

const MonacoDiffViewer = lazy(() =>
	import("./MonacoDiffViewer").then((m) => ({ default: m.MonacoDiffViewer })),
);

export interface DiffViewerSectionProps {
	isImage: boolean;
	isMarkdown: boolean;
	showPreview: boolean;
	imageDiff: {
		originalUrl: string | null;
		modifiedUrl: string | null;
		loading: boolean;
	};
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	filePath: string;
	language: string;
	fontSize?: number;
	changeGroups: ChangeGroup[];
	commentRanges: { start: number; end: number | undefined }[];
	onContentChange: (content: string) => void;
	onStageHunk: (groupIndex: number) => Promise<void>;
	onUnstageHunk: ((groupIndex: number) => Promise<void>) | undefined;
	onAddComment: (lineNumber: number, content: string, endLine?: number) => void;
	onAddEntry?: (threadId: string, content: string) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onImplementThread?: (threadId: string) => void;
	onPostToPr?: (threadId: string) => void;
	aiRunningThreadIds?: Set<string>;
	aiTaskThreadIds?: Set<string>;
	onOpenThreadAIModal?: (threadId?: string) => void;
	onAskAI?: (threadId: string) => void;
	onUpdateEntry?: (threadId: string, entryId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
	getThreadsForLine: (lineNumber: number) => Thread[];
	revealLine: { line: number; key: number; openThread?: boolean } | undefined;
	theme?: Theme;
	onSearchOccurrences?: (text: string) => void;
}

export function DiffViewerSection({
	isImage,
	isMarkdown,
	showPreview,
	imageDiff,
	originalContent,
	modifiedContent,
	diffMode,
	filePath,
	language,
	fontSize,
	changeGroups,
	commentRanges,
	onContentChange,
	onStageHunk,
	onUnstageHunk,
	onAddComment,
	onAddEntry,
	onDeleteThread,
	onResolveThread,
	onImplementThread,
	onPostToPr,
	aiRunningThreadIds,
	aiTaskThreadIds,
	onOpenThreadAIModal,
	onAskAI,
	onUpdateEntry,
	onCopyThread,
	getThreadsForLine,
	revealLine,
	theme,
	onSearchOccurrences,
}: DiffViewerSectionProps) {
	if (isImage) {
		return (
			<ImageDiffViewer
				originalUrl={imageDiff.originalUrl}
				modifiedUrl={imageDiff.modifiedUrl}
				loading={imageDiff.loading}
			/>
		);
	}

	if (isMarkdown && showPreview) {
		return (
			<MarkdownDiffViewer
				originalContent={originalContent}
				modifiedContent={modifiedContent}
				diffMode={diffMode}
			/>
		);
	}

	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full text-muted-foreground text-xs">
					Loading editor...
				</div>
			}
		>
			<MonacoDiffViewer
				key={filePath}
				originalContent={originalContent}
				modifiedContent={modifiedContent}
				language={language}
				diffMode={diffMode}
				onContentChange={onContentChange}
				fontSize={fontSize}
				changeGroups={changeGroups}
				commentRanges={commentRanges}
				onStageHunk={onStageHunk}
				onUnstageHunk={onUnstageHunk}
				onAddComment={onAddComment}
				onAddEntry={onAddEntry}
				onDeleteThread={onDeleteThread}
				onResolveThread={onResolveThread}
				onImplementThread={onImplementThread}
				onPostToPr={onPostToPr}
				aiRunningThreadIds={aiRunningThreadIds}
				aiTaskThreadIds={aiTaskThreadIds}
				onOpenThreadAIModal={onOpenThreadAIModal}
				onAskAI={onAskAI}
				onUpdateEntry={onUpdateEntry}
				onCopyThread={onCopyThread}
				getThreadsForLine={getThreadsForLine}
				revealLine={revealLine}
				theme={theme}
				filePath={filePath}
				onSearchOccurrences={onSearchOccurrences}
			/>
		</Suspense>
	);
}
