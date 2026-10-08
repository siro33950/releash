export interface Hunk {
	index: number;
	hunkId: string;
	oldStart: number;
	oldLines: number;
	newStart: number;
	newLines: number;
	lines: string[];
}

export interface ChangeGroup {
	groupIndex: number;
	groupId: string;
	hunkIndex: number;
	newStart: number;
	newEnd: number;
	lineOffsetStart: number;
	lineOffsetEnd: number;
	isStaged?: boolean;
}
