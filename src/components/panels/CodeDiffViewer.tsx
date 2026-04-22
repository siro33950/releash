import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { DiffComment } from "@/types/diffComment";
import type { DiffMode } from "@/types/settings";
import { ShikiDiffViewer } from "./ShikiDiffViewer";

interface DiffHunksResult {
	hunks: Hunk[];
}

export interface CodeDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	language?: string;
	filePath?: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
	comments?: DiffComment[];
	onAddComment?: (lineNumber: number, content: string) => Promise<void>;
	onAddRangeComment?: (
		startLine: number,
		endLine: number,
		content: string,
	) => Promise<void>;
	onUpdateComment?: (commentId: string, content: string) => Promise<void>;
	onDeleteComment?: (commentId: string) => Promise<void>;
	onSendComment?: (commentIds: string[]) => Promise<void>;
	scrollToLine?: number | null;
}

export function CodeDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	diffOnlyMode,
	language,
	filePath,
	changeGroups,
	onStageGroup,
	groupActionLabel,
	comments,
	onAddComment,
	onAddRangeComment,
	onUpdateComment,
	onDeleteComment,
	onSendComment,
	scrollToLine,
}: CodeDiffViewerProps) {
	const [detectedLanguage, setDetectedLanguage] = useState("plaintext");
	const [hunks, setHunks] = useState<Hunk[] | null>(null);
	const requestIdRef = useRef(0);
	const prevFilePathRef = useRef(filePath);

	useEffect(() => {
		const id = ++requestIdRef.current;
		if (prevFilePathRef.current !== filePath) {
			setHunks(null);
			prevFilePathRef.current = filePath;
		}
		if (!language) setDetectedLanguage("plaintext");

		const langPromise =
			!language && filePath
				? invoke<string>("get_language_from_path", { filePath }).catch(
						() => "plaintext",
					)
				: Promise.resolve(language ?? "plaintext");

		const hunksPromise = invoke<DiffHunksResult>("compute_diff_hunks", {
			original: originalContent,
			modified: modifiedContent,
			filePath: filePath ?? null,
		});

		Promise.all([langPromise, hunksPromise])
			.then(([detectedLang, hunksResult]) => {
				if (requestIdRef.current !== id) return;
				setDetectedLanguage(detectedLang);
				setHunks(hunksResult.hunks);
			})
			.catch(() => {
				if (requestIdRef.current !== id) return;
				setHunks([]);
			});
	}, [originalContent, modifiedContent, filePath, language]);

	const resolvedLanguage = language ?? detectedLanguage;

	if (hunks === null) {
		return null;
	}

	return (
		<ShikiDiffViewer
			originalContent={originalContent}
			modifiedContent={modifiedContent}
			diffMode={diffMode}
			diffOnlyMode={diffOnlyMode}
			language={resolvedLanguage}
			hunks={hunks}
			filePath={filePath}
			changeGroups={changeGroups}
			onStageGroup={onStageGroup}
			groupActionLabel={groupActionLabel}
			comments={comments}
			onAddComment={onAddComment}
			onAddRangeComment={onAddRangeComment}
			onUpdateComment={onUpdateComment}
			onDeleteComment={onDeleteComment}
			onSendComment={onSendComment}
			scrollToLine={scrollToLine}
		/>
	);
}
