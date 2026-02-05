export type FileStatus = "modified" | "added" | "deleted" | "untracked" | "ignored" | null;

export interface FileNode {
	name: string;
	path: string;
	type: "file" | "folder";
	status?: FileStatus;
	children?: FileNode[];
	isExpanded?: boolean;
}
