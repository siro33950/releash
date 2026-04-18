export interface Hunk {
	index: number;
	oldStart: number;
	oldLines: number;
	newStart: number;
	newLines: number;
	lines: string[];
}

export interface ChangeGroup {
	groupIndex: number;
	hunkIndex: number;
	newStart: number;
	newEnd: number;
	lineOffsetStart: number;
	lineOffsetEnd: number;
	isStaged?: boolean;
}
