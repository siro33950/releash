export interface CommentRange {
	start: number;
	end?: number;
}

export type CommentSeverity = "info" | "warning" | "error" | "suggestion";

export type CommentTarget = "local";

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

function normalizeStatus(value: string): "unsent" | "sent" {
	return value === "sent" ? "sent" : "unsent";
}

function normalizeTarget(_value: string): CommentTarget {
	return "local";
}

function normalizeSeverity(value: string): CommentSeverity | undefined {
	if (
		value === "info" ||
		value === "warning" ||
		value === "error" ||
		value === "suggestion"
	)
		return value;
	return undefined;
}

export function dtoToLineComment(dto: CommentItemDTO): LineComment {
	return {
		id: dto.id,
		filePath: dto.file_path,
		lineNumber: dto.line_number,
		...(dto.end_line != null && { endLine: dto.end_line }),
		content: dto.content,
		status: normalizeStatus(dto.status),
		createdAt: dto.created_at,
		...(dto.parent_id != null && { parentId: dto.parent_id }),
		...(dto.severity != null && {
			severity: normalizeSeverity(dto.severity),
		}),
		resolved: dto.resolved,
		target: normalizeTarget(dto.target),
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
