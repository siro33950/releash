import { lazy, Suspense } from "react";
import type { ChangeGroup } from "@/lib/computeHunks";
import type { LineComment } from "@/types/comment";
import type { DiffMode, Theme } from "@/types/settings";
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
	onDeleteComment?: (id: string) => void;
	onUpdateComment?: (id: string, content: string) => void;
	onCopyComment?: (comment: LineComment) => void;
	getCommentsForLine: (lineNumber: number) => LineComment[];
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
	onDeleteComment,
	onUpdateComment,
	onCopyComment,
	getCommentsForLine,
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
				onDeleteComment={onDeleteComment}
				onUpdateComment={onUpdateComment}
				onCopyComment={onCopyComment}
				getCommentsForLine={getCommentsForLine}
				revealLine={revealLine}
				theme={theme}
				filePath={filePath}
				onSearchOccurrences={onSearchOccurrences}
			/>
		</Suspense>
	);
}
