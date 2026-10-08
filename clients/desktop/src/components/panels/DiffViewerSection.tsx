import type { ReactNode } from "react";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import type { ReviewBinaryView, ReviewFallbackView } from "@/types/review";
import type { DiffMode } from "@/types/settings";
import { CodeDiffViewer } from "./CodeDiffViewer";
import { ImageDiffViewer } from "./ImageDiffViewer";
import { MarkdownDiffViewer } from "./MarkdownDiffViewer";

export interface DiffViewerSectionProps {
	isImage: boolean;
	isMarkdown: boolean;
	showPreview: boolean;
	imageDiff: {
		originalUrl: string | null;
		modifiedUrl: string | null;
		loading: boolean;
	};
	binaryView?: ReviewBinaryView | null;
	fallbackView?: ReviewFallbackView | null;
	error?: string | null;
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	filePath?: string;
	hunks: Hunk[] | null;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupId: string) => void;
	groupActionLabel?: string;
	comments?: ReviewDiscussionThread[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onAppendComment?: (threadId: string, content: string) => Promise<void>;
	onResolveThread?: (
		threadId: string,
		outcome: string,
		summary: string,
	) => Promise<void>;
	onDeleteThread?: (threadId: string) => Promise<void>;
	scrollToLine?: number | null;
	scrollToThread?: string | null;
	onLineRangeSelected?: (startLine: number, endLine: number) => void;
}

function formatFallbackReason(reason: ReviewFallbackView["reason"]): string {
	switch (reason) {
		case "fileSize":
			return "File is too large to preview";
		case "lineCount":
			return "File has too many lines to preview";
		case "hunkCount":
			return "Diff has too many hunks to preview";
		case "tokenization":
			return "File is too large to tokenize";
	}
}

function ReviewNotice({ children }: { children: ReactNode }) {
	return (
		<div className="flex h-full items-center justify-center bg-background p-4 text-sm text-muted-foreground">
			{children}
		</div>
	);
}

export function DiffViewerSection(props: DiffViewerSectionProps) {
	if (props.error) {
		return <ReviewNotice>{props.error}</ReviewNotice>;
	}

	if (props.fallbackView) {
		return (
			<ReviewNotice>
				{formatFallbackReason(props.fallbackView.reason)}
			</ReviewNotice>
		);
	}

	if (props.binaryView) {
		return <ReviewNotice>Binary file</ReviewNotice>;
	}

	if (props.isImage) {
		return (
			<ImageDiffViewer
				originalUrl={props.imageDiff.originalUrl}
				modifiedUrl={props.imageDiff.modifiedUrl}
				loading={props.imageDiff.loading}
			/>
		);
	}

	if (props.isMarkdown && props.showPreview) {
		return (
			<MarkdownDiffViewer
				originalContent={props.originalContent}
				modifiedContent={props.modifiedContent}
				diffMode={props.diffMode}
				diffOnlyMode={props.diffOnlyMode}
			/>
		);
	}

	if (!props.hunks) {
		return <ReviewNotice>Loading diff</ReviewNotice>;
	}

	return (
		<CodeDiffViewer
			originalContent={props.originalContent}
			modifiedContent={props.modifiedContent}
			diffMode={props.diffMode}
			diffOnlyMode={props.diffOnlyMode}
			filePath={props.filePath}
			hunks={props.hunks}
			changeGroups={props.changeGroups}
			onStageGroup={props.onStageGroup}
			groupActionLabel={props.groupActionLabel}
			comments={props.comments}
			onAddComment={props.onAddComment}
			onAddRangeComment={props.onAddRangeComment}
			onAppendComment={props.onAppendComment}
			onResolveThread={props.onResolveThread}
			onDeleteThread={props.onDeleteThread}
			scrollToLine={props.scrollToLine}
			scrollToThread={props.scrollToThread}
			onLineRangeSelected={props.onLineRangeSelected}
		/>
	);
}
