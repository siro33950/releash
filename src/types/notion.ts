export interface NotionTask {
	id: string;
	title: string;
	url: string;
	labels: Record<string, string[]>;
	branch_name: string;
	created_at: string;
	last_edited_at: string;
}

export interface NotionTaskPage {
	tasks: NotionTask[];
	has_more: boolean;
	next_cursor: string | null;
}

export interface LabelProperty {
	name: string;
	property_type: string;
}

export interface PropertyMapping {
	title: string;
	labels: LabelProperty[];
	branch_name: string;
	branch_prefix: string;
}

export interface NotionRepoConfig {
	api_token: string;
	database_id: string;
	property_mapping: PropertyMapping;
}

type NotionConfigStatus =
	| "not_configured"
	| "configured"
	| "invalid_token"
	| "invalid_database"
	| "network_error";

export interface NotionPropertyInfo {
	name: string;
	property_type: string;
	options: string[];
}

export interface NotionLabelOption {
	property_name: string;
	property_type: string;
	options: string[];
	option_ids: string[];
}

export interface NotionValidationResult {
	status: NotionConfigStatus;
	properties: NotionPropertyInfo[];
}
