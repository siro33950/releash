export type DiffRangeType = "added" | "modified" | "deleted";

export interface DiffRange {
	startLine: number;
	endLine: number;
	type: DiffRangeType;
}

export interface SplitRow {
	left: string | null;
	right: string | null;
	type: "unchanged" | "added" | "removed" | "modified";
}

export interface InlineChunk {
	content: string;
	type: "unchanged" | "added" | "removed";
}
