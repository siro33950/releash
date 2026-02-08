export interface CommentRange {
	start: number;
	end?: number;
}

export interface LineComment {
	id: string;
	filePath: string;
	lineNumber: number;
	endLine?: number;
	content: string;
	status: "unsent" | "sent";
	createdAt: number;
}
