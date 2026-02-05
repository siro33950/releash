export interface TabInfo {
	path: string;
	name: string;
	content: string;
	originalContent: string;
	isDirty: boolean;
	language: string;
	eol: "LF" | "CRLF";
}
