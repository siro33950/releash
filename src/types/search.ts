export interface SearchMatch {
	path: string;
	line_number: number;
	line_content: string;
	match_start: number;
	match_end: number;
}

export interface SearchResult {
	matches: SearchMatch[];
	total_matches: number;
	truncated: boolean;
}

export interface SearchOptions {
	caseSensitive?: boolean;
	isRegex?: boolean;
	maxResults?: number;
}

export interface DefinitionLocation {
	path: string;
	line_number: number;
	column: number;
	line_content: string;
	kind: string;
}
