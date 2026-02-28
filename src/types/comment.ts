export interface CommentRange {
	start: number;
	end?: number;
}

export type CommentSeverity = "info" | "warning" | "error" | "suggestion";

export type CommentTarget = "ai" | "review" | "local";

export interface LineComment {
	id: string;
	filePath: string;
	lineNumber: number;
	endLine?: number;
	content: string;
	status: "unsent" | "sent";
	createdAt: number;
	parentId?: string;
	severity?: CommentSeverity;
	resolved: boolean;
	target: CommentTarget;
}

export interface CommentItemDTO {
	id: string;
	file_path: string;
	line_number: number;
	end_line?: number;
	content: string;
	status: string;
	created_at: number;
	parent_id?: string;
	severity?: string;
	resolved: boolean;
	target: string;
}

export function dtoToLineComment(dto: CommentItemDTO): LineComment {
	return {
		id: dto.id,
		filePath: dto.file_path,
		lineNumber: dto.line_number,
		...(dto.end_line != null && { endLine: dto.end_line }),
		content: dto.content,
		status: dto.status as "unsent" | "sent",
		createdAt: dto.created_at,
		...(dto.parent_id != null && { parentId: dto.parent_id }),
		...(dto.severity != null && {
			severity: dto.severity as CommentSeverity,
		}),
		resolved: dto.resolved,
		target: dto.target as CommentTarget,
	};
}

export function lineCommentToDTO(c: LineComment): CommentItemDTO {
	return {
		id: c.id,
		file_path: c.filePath,
		line_number: c.lineNumber,
		...(c.endLine != null && { end_line: c.endLine }),
		content: c.content,
		status: c.status,
		created_at: c.createdAt,
		...(c.parentId != null && { parent_id: c.parentId }),
		...(c.severity != null && { severity: c.severity }),
		resolved: c.resolved,
		target: c.target,
	};
}
