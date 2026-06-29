import type { ChangeGroup, Hunk } from "@/lib/computeHunks";
import type { GitFileStatus } from "@/types/git";
import type { DiffBase } from "@/types/settings";

export interface DiffTreeNode {
	id: string;
	name: string;
	path: string;
	node_type: "file" | "folder";
	status: string | null;
	additions: number | null;
	deletions: number | null;
	children: DiffTreeNode[];
}

interface ReviewFileEntry {
	fileId: string;
	path: string;
	indexStatus: string;
	worktreeStatus: string;
	additions: number;
	deletions: number;
}

interface ReviewDiffStat {
	path: string;
	index_additions: number;
	index_deletions: number;
	wt_additions: number;
	wt_deletions: number;
}

export interface ReviewSnapshot {
	version: number;
	stale: boolean;
	loading: boolean;
	limited: boolean;
	base: DiffBase;
	files: ReviewFileEntry[];
	stagedFiles: GitFileStatus[];
	changedFiles: GitFileStatus[];
	diffStats: ReviewDiffStat[];
	tree: DiffTreeNode[];
	stagedTree: DiffTreeNode[];
	changesTree: DiffTreeNode[];
	stagedFileCount: number;
	changesFileCount: number;
}

export interface ReviewViewport {
	startLine: number;
	endLine: number;
}

type ReviewTextSource = "diff" | "added" | "deleted";
type ReviewLimitReason =
	| "fileSize"
	| "lineCount"
	| "hunkCount"
	| "tokenization";

interface ReviewTextDiffView {
	kind: "textDiff";
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	original: string;
	modified: string;
	source: ReviewTextSource;
	hunks: Hunk[];
	changeGroups: ChangeGroup[];
	limited: boolean;
	viewport: ReviewViewport | null;
	totalLines: number;
}

interface ReviewImageView {
	kind: "image";
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	originalUrl: string | null;
	modifiedUrl: string | null;
	mime: string;
}

export interface ReviewBinaryView {
	kind: "binary";
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	originalUrl: string | null;
	modifiedUrl: string | null;
	originalSize: number | null;
	modifiedSize: number | null;
}

export interface ReviewFallbackView {
	kind: "fallback";
	version: number;
	stale: boolean;
	fileId: string;
	path: string;
	reason: ReviewLimitReason;
	totalLines: number | null;
	sizeBytes: number | null;
	hunkCount: number | null;
	limited: true;
}

export type ReviewFileView =
	| ReviewTextDiffView
	| ReviewImageView
	| ReviewBinaryView
	| ReviewFallbackView;
