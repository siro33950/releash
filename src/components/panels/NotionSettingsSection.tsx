import { GitBranch, Loader2, Trash2 } from "lucide-react";
import { Fragment, useCallback } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Message } from "@/components/ui/message";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import type { NotionRepoDraft } from "@/hooks/useNotionSettings";
import type { LabelProperty, NotionPropertyInfo } from "@/types/notion";

const labelClass = "text-xs font-medium text-muted-foreground";

interface NotionSettingsSectionProps {
	repoPaths: string[];
	drafts: Map<string, NotionRepoDraft>;
	updateDraft: (
		repoPath: string,
		updater: (d: NotionRepoDraft) => NotionRepoDraft,
	) => void;
	validate: (repoPath: string) => Promise<void>;
	markForDelete: (repoPath: string) => void;
}

export function NotionSettingsSection({
	repoPaths,
	drafts,
	updateDraft,
	validate,
	markForDelete,
}: NotionSettingsSectionProps) {
	if (repoPaths.length === 0) {
		return (
			<p className="text-xs text-muted-foreground">
				No repositories registered.
			</p>
		);
	}

	return (
		<div className="flex flex-col">
			{repoPaths.map((repoPath, i) => {
				const draft = drafts.get(repoPath);
				if (!draft) return null;
				return (
					<Fragment key={repoPath}>
						{i > 0 && <Separator className="my-3" />}
						<NotionRepoConfigItem
							repoPath={repoPath}
							draft={draft}
							updateDraft={updateDraft}
							validate={validate}
							markForDelete={markForDelete}
						/>
					</Fragment>
				);
			})}
		</div>
	);
}

interface NotionRepoConfigItemProps {
	repoPath: string;
	draft: NotionRepoDraft;
	updateDraft: (
		repoPath: string,
		updater: (d: NotionRepoDraft) => NotionRepoDraft,
	) => void;
	validate: (repoPath: string) => Promise<void>;
	markForDelete: (repoPath: string) => void;
}

function NotionRepoConfigItem({
	repoPath,
	draft,
	updateDraft,
	validate,
	markForDelete,
}: NotionRepoConfigItemProps) {
	const name = repoPath.split(/[\\/]/).pop() ?? repoPath;

	const handleValidate = useCallback(() => {
		validate(repoPath);
	}, [repoPath, validate]);

	const handleMarkForDelete = useCallback(() => {
		markForDelete(repoPath);
	}, [repoPath, markForDelete]);

	return (
		<div className={draft.markedForDelete ? "opacity-50" : ""}>
			<div className="flex items-center gap-2 py-2 text-sm font-medium">
				<GitBranch className="size-3.5 shrink-0 text-muted-foreground" />
				<span className="font-mono truncate">{name}</span>
				{(draft.apiToken || draft.databaseId) && (
					<Button
						variant={draft.markedForDelete ? "outline" : "ghost"}
						size="sm"
						className="ml-auto h-6 px-2"
						onClick={handleMarkForDelete}
						aria-label={
							draft.markedForDelete
								? "Undo delete Notion configuration"
								: "Delete Notion configuration"
						}
					>
						{draft.markedForDelete ? (
							<span className="text-[10px]">Undo</span>
						) : (
							<Trash2 className="size-3 text-destructive" aria-hidden="true" />
						)}
					</Button>
				)}
			</div>

			{draft.markedForDelete ? (
				<p className="text-xs text-muted-foreground ml-5 pl-3">
					This configuration will be deleted on save.
				</p>
			) : (
				<div className="ml-5 pl-3 flex flex-col gap-2">
					<div className="flex flex-col gap-1.5">
						<label htmlFor={`notion-token-${repoPath}`} className={labelClass}>
							API Token
						</label>
						<Input
							id={`notion-token-${repoPath}`}
							type="password"
							variant="panel"
							size="sm"
							value={draft.apiToken}
							onChange={(e) =>
								updateDraft(repoPath, (d) => ({
									...d,
									apiToken: e.target.value,
								}))
							}
							placeholder="ntn_..."
						/>
					</div>

					<div className="flex flex-col gap-1.5">
						<label htmlFor={`notion-db-${repoPath}`} className={labelClass}>
							Database ID
						</label>
						<Input
							id={`notion-db-${repoPath}`}
							type="text"
							variant="panel"
							size="sm"
							value={draft.databaseId}
							onChange={(e) =>
								updateDraft(repoPath, (d) => ({
									...d,
									databaseId: e.target.value,
								}))
							}
							placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
						/>
					</div>

					<Button
						variant="outline"
						size="sm"
						onClick={handleValidate}
						disabled={draft.validating || !draft.apiToken || !draft.databaseId}
					>
						{draft.validating ? (
							<Loader2 className="size-3.5 mr-1 animate-spin" />
						) : null}
						Test Connection
					</Button>

					{draft.validationStatus && draft.validationStatus !== "success" && (
						<Message message={draft.validationStatus} size="xs" />
					)}
					{draft.validationStatus === "success" && (
						<Message
							severity="success"
							message="Connection successful"
							size="xs"
						/>
					)}

					{draft.properties.length > 0 && (
						<PropertyMappingSection
							repoPath={repoPath}
							draft={draft}
							updateDraft={updateDraft}
						/>
					)}
				</div>
			)}
		</div>
	);
}

function PropertyMappingSection({
	repoPath,
	draft,
	updateDraft,
}: {
	repoPath: string;
	draft: NotionRepoDraft;
	updateDraft: (
		repoPath: string,
		updater: (d: NotionRepoDraft) => NotionRepoDraft,
	) => void;
}) {
	const { properties, propertyMapping: mapping } = draft;

	const handleToggleLabel = useCallback(
		(prop: NotionPropertyInfo) => {
			updateDraft(repoPath, (d) => {
				const current = d.propertyMapping.labels;
				const exists = current.some((s) => s.name === prop.name);
				const labels: LabelProperty[] = exists
					? current.filter((s) => s.name !== prop.name)
					: [
							...current,
							{ name: prop.name, property_type: prop.property_type },
						];
				return {
					...d,
					propertyMapping: { ...d.propertyMapping, labels },
				};
			});
		},
		[repoPath, updateDraft],
	);

	return (
		<div className="flex flex-col gap-2">
			<span className={labelClass}>Property Mapping</span>

			<div className="flex flex-col gap-1.5">
				<span className="text-[10px] text-muted-foreground">Title</span>
				<Select
					value={mapping.title}
					onValueChange={(value) =>
						updateDraft(repoPath, (d) => ({
							...d,
							propertyMapping: {
								...d.propertyMapping,
								title: value,
							},
						}))
					}
				>
					<SelectTrigger size="sm" className="w-full text-xs">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{properties.map((prop) => (
							<SelectItem key={prop.name} value={prop.name}>
								{prop.name} ({prop.property_type})
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			<div className="flex flex-col gap-1">
				<span className="text-[10px] text-muted-foreground">Labels</span>
				<div className="flex flex-col gap-0.5 ml-1">
					{properties.map((prop) => (
						<div key={prop.name} className="flex items-center gap-1.5">
							<Checkbox
								id={`notion-label-${repoPath}-${prop.name}`}
								checked={mapping.labels.some((s) => s.name === prop.name)}
								onCheckedChange={() => handleToggleLabel(prop)}
							/>
							<label
								htmlFor={`notion-label-${repoPath}-${prop.name}`}
								className="text-[10px] cursor-pointer"
							>
								{prop.name} ({prop.property_type})
							</label>
						</div>
					))}
				</div>
			</div>

			<div className="flex flex-col gap-1.5">
				<span className="text-[10px] text-muted-foreground">Branch Name</span>
				<Select
					value={mapping.branch_name}
					onValueChange={(value) =>
						updateDraft(repoPath, (d) => ({
							...d,
							propertyMapping: {
								...d.propertyMapping,
								branch_name: value,
							},
						}))
					}
				>
					<SelectTrigger size="sm" className="w-full text-xs">
						<SelectValue placeholder="(Not set)" />
					</SelectTrigger>
					<SelectContent>
						{properties.map((prop) => (
							<SelectItem key={prop.name} value={prop.name}>
								{prop.name} ({prop.property_type})
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			<div className="flex flex-col gap-1.5">
				<label
					htmlFor={`notion-prefix-${repoPath}`}
					className="text-[10px] text-muted-foreground"
				>
					Prefix
				</label>
				<Input
					id={`notion-prefix-${repoPath}`}
					type="text"
					variant="panel"
					size="sm"
					value={mapping.branch_prefix}
					onChange={(e) =>
						updateDraft(repoPath, (d) => ({
							...d,
							propertyMapping: {
								...d.propertyMapping,
								branch_prefix: e.target.value,
							},
						}))
					}
					placeholder="feat/"
				/>
			</div>
		</div>
	);
}
