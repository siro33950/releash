// FacetKind → dir_name マッピング。
// Rust FacetKind::dir_name() (src-tauri/src/workflow/facet.rs) と同期が必要。
// DiagnosticReport のキー形式 "{dir_name}/{key}" との変換に使用。
export const FACET_KIND_DIR_MAP: Record<string, string> = {
	policy: "policies",
	knowledge: "knowledge",
	instruction: "instructions",
	output_contract: "output_contracts",
};

export function facetKindToDirName(kind: string): string {
	return FACET_KIND_DIR_MAP[kind] ?? kind;
}

export function extractTemplateVariables(content: string): string[] {
	const matches = content.match(/\{\{([^}]+)\}\}/g);
	if (!matches) return [];
	return [
		...new Set(matches.map((m) => m.slice(2, -2).trim()).filter((v) => v)),
	];
}
