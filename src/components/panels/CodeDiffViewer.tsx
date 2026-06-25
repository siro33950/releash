import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import type { DiffMode } from "@/types/settings";
import { ShikiDiffViewer } from "./ShikiDiffViewer";

export interface CodeDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	language?: string;
	filePath?: string;
	hunks: Hunk[];
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

export function CodeDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	diffOnlyMode,
	language,
	filePath,
	hunks: providedHunks,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	comments,
	onAddComment,
	onAddRangeComment,
	onAppendComment,
	onResolveThread,
	onDeleteThread,
	scrollToLine,
	scrollToThread,
	onLineRangeSelected,
}: CodeDiffViewerProps) {
	const [detectedLanguage, setDetectedLanguage] = useState("plaintext");

	useEffect(() => {
		if (language) {
			setDetectedLanguage(language);
			return;
		}
		if (!filePath) {
			setDetectedLanguage("plaintext");
			return;
		}

		let cancelled = false;
		setDetectedLanguage("plaintext");
		invoke<string>("get_language_from_path", { filePath })
			.then((detectedLang) => {
				if (!cancelled) setDetectedLanguage(detectedLang);
			})
			.catch(() => {
				if (!cancelled) setDetectedLanguage("plaintext");
			});
		return () => {
			cancelled = true;
		};
	}, [filePath, language]);

	const resolvedLanguage = language ?? detectedLanguage;

	return (
		<ShikiDiffViewer
			originalContent={originalContent}
			modifiedContent={modifiedContent}
			diffMode={diffMode}
			diffOnlyMode={diffOnlyMode}
			language={resolvedLanguage}
			hunks={providedHunks}
			filePath={filePath}
			changeGroups={changeGroups}
			onStageGroup={onStageGroup}
			groupActionLabel={groupActionLabel}
			comments={comments}
			onAddComment={onAddComment}
			onAddRangeComment={onAddRangeComment}
			onAppendComment={onAppendComment}
			onResolveThread={onResolveThread}
			onDeleteThread={onDeleteThread}
			scrollToLine={scrollToLine}
			scrollToThread={scrollToThread}
			onLineRangeSelected={onLineRangeSelected}
		/>
	);
}
