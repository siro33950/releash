export interface DiffComment {
	id: string;
	filePath: string;
	lineNumber?: number;
	endLine?: number;
	content: string;
	status: "unsent" | "sent";
	createdAt: number;
}
