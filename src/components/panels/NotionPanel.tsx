import { invoke } from "@tauri-apps/api/core";
import {
	ChevronDown,
	ChevronRight,
	ExternalLink,
	GitBranch,
	Loader2,
	RefreshCw,
	Settings,
	Trash2,
} from "lucide-react";
import { useCallback, useState } from "react";
import { EmptyState } from "@/components/panels/EmptyState";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useNotionConfig } from "@/hooks/useNotionConfig";
import { useNotionLabelOptions } from "@/hooks/useNotionLabelOptions";
import { useNotionTasks } from "@/hooks/useNotionTasks";
import { generateNotionBranchName } from "@/lib/notionBranch";
import { branchToDir, computeWorktreeDir } from "@/lib/worktreePath";
import type { WorktreeEntry } from "@/types/git";
import type {
	LabelProperty,
	NotionPropertyInfo,
	NotionTask,
	NotionValidationResult,
	PropertyMapping,
} from "@/types/notion";

interface NotionPanelProps {
	repoPaths: string[];
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
}

export function NotionPanel({ repoPaths, onSelectWorktree }: NotionPanelProps) {
	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Notion Tasks
				</span>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				{repoPaths.map((repoPath) => (
					<NotionRepoSection
						key={repoPath}
						repoPath={repoPath}
						onSelectWorktree={onSelectWorktree}
						defaultExpanded={repoPaths.length === 1}
					/>
				))}
				{repoPaths.length === 0 && (
					<EmptyState
						compact
						title="No repositories"
						className="px-3 py-4 text-center"
					/>
				)}
			</ScrollArea>
		</div>
	);
}

interface NotionRepoSectionProps {
	repoPath: string;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	defaultExpanded: boolean;
}

function NotionRepoSection({
	repoPath,
	onSelectWorktree,
	defaultExpanded,
}: NotionRepoSectionProps) {
	const [expanded, setExpanded] = useState(defaultExpanded);
	const [showConfig, setShowConfig] = useState(false);
	const {
		config,
		loading: configLoading,
		save,
		remove,
		validate,
		isConfigured,
	} = useNotionConfig(repoPath);
	const repoName = repoPath.split("/").filter(Boolean).pop() ?? "repo";

	return (
		<div className="border-b border-border">
			<button
				type="button"
				className="flex items-center gap-1.5 w-full px-3 py-1.5 text-xs font-medium hover:bg-accent/50"
				onClick={() => setExpanded((v) => !v)}
			>
				{expanded ? (
					<ChevronDown className="size-3 shrink-0" />
				) : (
					<ChevronRight className="size-3 shrink-0" />
				)}
				<span className="truncate">{repoName}</span>
				{configLoading && <Loader2 className="size-3 animate-spin ml-auto" />}
			</button>
			{expanded && (
				<div className="pb-1">
					{!isConfigured && !configLoading && !showConfig && (
						<div className="px-3 py-2">
							<div className="text-[10px] text-muted-foreground mb-2">
								Notion連携が未設定です
							</div>
							<Button
								variant="outline"
								size="sm"
								className="w-full h-6 text-[10px]"
								onClick={() => setShowConfig(true)}
							>
								<Settings className="size-3 mr-1" />
								設定する
							</Button>
						</div>
					)}
					{showConfig && (
						<NotionConfigForm
							initialConfig={config}
							onSave={async (apiToken, databaseId, mapping) => {
								await save(apiToken, databaseId, mapping);
								setShowConfig(false);
							}}
							onCancel={() => setShowConfig(false)}
							onDelete={
								isConfigured
									? async () => {
											await remove();
											setShowConfig(false);
										}
									: undefined
							}
							validate={validate}
						/>
					)}
					{isConfigured && !showConfig && (
						<NotionTaskList
							repoPath={repoPath}
							repoName={repoName}
							onSelectWorktree={onSelectWorktree}
							onShowConfig={() => setShowConfig(true)}
							branchPrefix={config?.property_mapping.branch_prefix ?? ""}
						/>
					)}
				</div>
			)}
		</div>
	);
}

interface NotionConfigFormProps {
	initialConfig: {
		api_token: string;
		database_id: string;
		property_mapping: PropertyMapping;
	} | null;
	onSave: (
		apiToken: string,
		databaseId: string,
		mapping: PropertyMapping,
	) => Promise<void>;
	onCancel: () => void;
	onDelete?: () => Promise<void>;
	validate: (
		apiToken: string,
		databaseId: string,
	) => Promise<NotionValidationResult>;
}

function NotionConfigForm({
	initialConfig,
	onSave,
	onCancel,
	onDelete,
	validate,
}: NotionConfigFormProps) {
	const [apiToken, setApiToken] = useState(initialConfig?.api_token ?? "");
	const [databaseId, setDatabaseId] = useState(
		initialConfig?.database_id ?? "",
	);
	const [mapping, setMapping] = useState<PropertyMapping>(
		initialConfig?.property_mapping ?? {
			title: "Name",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		},
	);
	const [validating, setValidating] = useState(false);
	const [saving, setSaving] = useState(false);
	const [properties, setProperties] = useState<NotionPropertyInfo[]>([]);
	const [validationStatus, setValidationStatus] = useState<string | null>(null);
	const [deleting, setDeleting] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const [deleteError, setDeleteError] = useState<string | null>(null);

	const handleValidate = useCallback(async () => {
		setValidating(true);
		setValidationStatus(null);
		try {
			const result = await validate(apiToken, databaseId);
			setProperties(result.properties);
			if (result.status === "configured") {
				setValidationStatus("success");
			} else if (result.status === "invalid_token") {
				setValidationStatus("APIトークンが無効です");
			} else if (result.status === "invalid_database") {
				setValidationStatus("データベースIDが無効です");
			} else if (result.status === "network_error") {
				setValidationStatus("ネットワークエラー: 接続を確認してください");
			} else {
				setValidationStatus("設定が不完全です");
			}
		} catch (e) {
			setValidationStatus(String(e));
		} finally {
			setValidating(false);
		}
	}, [apiToken, databaseId, validate]);

	const handleSave = useCallback(async () => {
		setSaving(true);
		setSaveError(null);
		try {
			await onSave(apiToken, databaseId, mapping);
		} catch (e) {
			setSaveError(String(e));
		} finally {
			setSaving(false);
		}
	}, [apiToken, databaseId, mapping, onSave]);

	const handleDelete = useCallback(async () => {
		if (!onDelete) return;
		setDeleting(true);
		setDeleteError(null);
		try {
			await onDelete();
		} catch (e) {
			setDeleteError(String(e));
		} finally {
			setDeleting(false);
		}
	}, [onDelete]);

	return (
		<div className="px-2 py-1.5 flex flex-col gap-1.5">
			{/* biome-ignore lint/a11y/noLabelWithoutControl: Input renders <input> inside label */}
			<label className="text-[10px] text-muted-foreground">
				API Token
				<Input
					type="password"
					variant="panel"
					size="xs"
					className="mt-0.5"
					value={apiToken}
					onChange={(e) => setApiToken(e.target.value)}
					placeholder="ntn_..."
				/>
			</label>
			{/* biome-ignore lint/a11y/noLabelWithoutControl: Input renders <input> inside label */}
			<label className="text-[10px] text-muted-foreground">
				Database ID
				<Input
					type="text"
					variant="panel"
					size="xs"
					className="mt-0.5"
					value={databaseId}
					onChange={(e) => setDatabaseId(e.target.value)}
					placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
				/>
			</label>

			<Button
				variant="outline"
				size="sm"
				className="w-full h-6 text-[10px]"
				onClick={handleValidate}
				disabled={validating || !apiToken || !databaseId}
			>
				{validating ? <Loader2 className="size-3 mr-1 animate-spin" /> : null}
				接続テスト
			</Button>

			{validationStatus && validationStatus !== "success" && (
				<div className="text-[10px] text-destructive">{validationStatus}</div>
			)}
			{validationStatus === "success" && (
				<div className="text-[10px] text-success">接続成功</div>
			)}

			{properties.length > 0 && (
				<div className="flex flex-col gap-1 mt-1">
					<div className="text-[10px] font-medium text-muted-foreground">
						プロパティマッピング
					</div>
					<PropertySelect
						label="タイトル"
						value={mapping.title}
						properties={properties}
						onChange={(v) => setMapping((m) => ({ ...m, title: v }))}
					/>
					<PropertyCheckboxGroup
						label="ラベル"
						selected={mapping.labels}
						properties={properties}
						onChange={(v) => setMapping((m) => ({ ...m, labels: v }))}
					/>
					<PropertySelect
						label="ブランチ名"
						value={mapping.branch_name}
						properties={properties}
						onChange={(v) => setMapping((m) => ({ ...m, branch_name: v }))}
						allowEmpty
					/>
					{/* biome-ignore lint/a11y/noLabelWithoutControl: Input renders <input> inside label */}
					<label className="text-[10px] text-muted-foreground flex items-center gap-1">
						<span className="w-16 shrink-0">プレフィックス</span>
						<Input
							type="text"
							variant="panel"
							size="xs"
							className="flex-1"
							value={mapping.branch_prefix}
							onChange={(e) =>
								setMapping((m) => ({ ...m, branch_prefix: e.target.value }))
							}
							placeholder="feat/"
						/>
					</label>
				</div>
			)}

			<div className="flex gap-1 mt-1">
				<Button
					variant="outline"
					size="sm"
					className="flex-1 h-6 text-[10px]"
					onClick={onCancel}
				>
					キャンセル
				</Button>
				{onDelete && (
					<Button
						variant="destructive"
						size="sm"
						className="h-6 text-[10px] px-2"
						onClick={handleDelete}
						disabled={deleting}
					>
						{deleting ? (
							<Loader2 className="size-3 animate-spin" />
						) : (
							<Trash2 className="size-3" />
						)}
					</Button>
				)}
				<Button
					size="sm"
					className="flex-1 h-6 text-[10px]"
					onClick={handleSave}
					disabled={saving || !apiToken || !databaseId}
				>
					{saving ? <Loader2 className="size-3 mr-1 animate-spin" /> : null}
					保存
				</Button>
			</div>
			{saveError && (
				<div className="text-[10px] text-destructive">{saveError}</div>
			)}
			{deleteError && (
				<div className="text-[10px] text-destructive">{deleteError}</div>
			)}
		</div>
	);
}

interface PropertySelectProps {
	label: string;
	value: string;
	properties: NotionPropertyInfo[];
	onChange: (value: string) => void;
	allowEmpty?: boolean;
}

function PropertySelect({
	label,
	value,
	properties,
	onChange,
	allowEmpty,
}: PropertySelectProps) {
	return (
		<label className="text-[10px] text-muted-foreground flex items-center gap-1">
			<span className="w-16 shrink-0">{label}</span>
			<select
				value={value}
				onChange={(e) => onChange(e.target.value)}
				className="flex-1 bg-muted border border-border rounded px-1 py-0.5 text-[10px] focus:outline-none focus:ring-1 focus:ring-primary"
			>
				{allowEmpty && <option value="">（未設定）</option>}
				{properties.map((prop) => (
					<option key={prop.name} value={prop.name}>
						{prop.name} ({prop.property_type})
					</option>
				))}
			</select>
		</label>
	);
}

interface PropertyCheckboxGroupProps {
	label: string;
	selected: LabelProperty[];
	properties: NotionPropertyInfo[];
	onChange: (value: LabelProperty[]) => void;
}

function PropertyCheckboxGroup({
	label,
	selected,
	properties,
	onChange,
}: PropertyCheckboxGroupProps) {
	const handleToggle = useCallback(
		(prop: NotionPropertyInfo) => {
			onChange(
				selected.some((s) => s.name === prop.name)
					? selected.filter((s) => s.name !== prop.name)
					: [
							...selected,
							{ name: prop.name, property_type: prop.property_type },
						],
			);
		},
		[selected, onChange],
	);

	return (
		<div className="text-[10px] text-muted-foreground">
			<span className="font-medium">{label}</span>
			<div className="flex flex-col gap-0.5 mt-0.5 ml-1">
				{properties.map((prop) => (
					<label
						key={prop.name}
						className="flex items-center gap-1 cursor-pointer"
					>
						<input
							type="checkbox"
							checked={selected.some((s) => s.name === prop.name)}
							onChange={() => handleToggle(prop)}
							className="size-3"
						/>
						<span>
							{prop.name} ({prop.property_type})
						</span>
					</label>
				))}
			</div>
		</div>
	);
}

interface NotionTaskListProps {
	repoPath: string;
	repoName: string;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	onShowConfig: () => void;
	branchPrefix: string;
}

function loadStoredFilters(repoPath: string): {
	title: string;
	labels: Record<string, string>;
} {
	try {
		const raw = localStorage.getItem(`notion-filters:${repoPath}`);
		if (raw) {
			const parsed = JSON.parse(raw);
			return {
				title: typeof parsed.title === "string" ? parsed.title : "",
				labels:
					parsed.labels && typeof parsed.labels === "object"
						? parsed.labels
						: {},
			};
		}
	} catch {
		// ignore
	}
	return { title: "", labels: {} };
}

function saveStoredFilters(
	repoPath: string,
	title: string,
	labels: Record<string, string>,
) {
	try {
		localStorage.setItem(
			`notion-filters:${repoPath}`,
			JSON.stringify({ title, labels }),
		);
	} catch {
		// ignore
	}
}

function NotionTaskList({
	repoPath,
	repoName,
	onSelectWorktree,
	onShowConfig,
	branchPrefix,
}: NotionTaskListProps) {
	const [stored] = useState(() => loadStoredFilters(repoPath));
	const { tasks, loading, hasMore, search, loadMore, refresh } = useNotionTasks(
		repoPath,
		stored,
	);
	const { labelOptions } = useNotionLabelOptions(repoPath);
	const [titleFilter, setTitleFilter] = useState(stored.title);
	const [labelFilters, setLabelFilters] = useState<Record<string, string>>(
		stored.labels,
	);

	const hasActiveFilters =
		titleFilter !== "" || Object.values(labelFilters).some((v) => v !== "");

	const handleTitleChange = useCallback(
		(value: string) => {
			setTitleFilter(value);
			saveStoredFilters(repoPath, value, labelFilters);
			search(value, labelFilters);
		},
		[repoPath, labelFilters, search],
	);

	const handleLabelChange = useCallback(
		(propertyName: string, value: string) => {
			const newFilters = { ...labelFilters, [propertyName]: value };
			setLabelFilters(newFilters);
			saveStoredFilters(repoPath, titleFilter, newFilters);
			search(titleFilter, newFilters);
		},
		[repoPath, titleFilter, labelFilters, search],
	);

	return (
		<>
			{(tasks.length > 0 || hasActiveFilters) && (
				<div className="px-2 py-1.5 flex flex-col gap-1">
					<Input
						type="text"
						variant="panel"
						size="xs"
						placeholder="Filter by title..."
						value={titleFilter}
						onChange={(e) => handleTitleChange(e.target.value)}
					/>
					{labelOptions.map((opt) => (
						<select
							key={opt.property_name}
							value={labelFilters[opt.property_name] ?? ""}
							onChange={(e) =>
								handleLabelChange(opt.property_name, e.target.value)
							}
							className="w-full bg-muted border border-border rounded px-2 py-1 text-[10px] focus:outline-none focus:ring-1 focus:ring-primary"
						>
							<option value="">{opt.property_name}: All</option>
							{opt.options.map((v, i) => (
								<option
									key={v}
									value={opt.option_ids.length > 0 ? opt.option_ids[i] : v}
								>
									{v}
								</option>
							))}
						</select>
					))}
				</div>
			)}
			{loading && tasks.length === 0 && (
				<div className="px-3 py-2 flex items-center justify-center">
					<Loader2 className="size-3 animate-spin" />
				</div>
			)}
			{!loading && tasks.length === 0 && (
				<EmptyState
					compact
					title="No tasks"
					className="px-3 py-2 text-[10px]"
				/>
			)}
			{tasks.map((task) => (
				<NotionTaskCard
					key={task.id}
					task={task}
					repoPath={repoPath}
					repoName={repoName}
					onSelectWorktree={onSelectWorktree}
					branchPrefix={branchPrefix}
				/>
			))}
			{hasMore && (
				<div className="px-3 py-1">
					<Button
						variant="outline"
						size="sm"
						className="w-full h-6 text-[10px]"
						onClick={loadMore}
						disabled={loading}
					>
						{loading ? <Loader2 className="size-3 mr-1 animate-spin" /> : null}
						Load more
					</Button>
				</div>
			)}
			{!loading && (
				<div className="px-3 pt-1 flex items-center gap-1">
					<Button
						variant="ghost"
						size="sm"
						className="h-5 px-1.5 text-[10px] text-muted-foreground"
						onClick={refresh}
					>
						<RefreshCw className="size-2.5 mr-1" />
						Refresh
					</Button>
					<Button
						variant="ghost"
						size="sm"
						className="h-5 px-1.5 text-[10px] text-muted-foreground ml-auto"
						onClick={onShowConfig}
					>
						<Settings className="size-2.5 mr-1" />
						設定
					</Button>
				</div>
			)}
		</>
	);
}

interface NotionTaskCardProps {
	task: NotionTask;
	repoPath: string;
	repoName: string;
	onSelectWorktree: (
		path: string,
		branchName?: string,
		repoName?: string,
	) => void;
	branchPrefix: string;
}

function NotionTaskCard({
	task,
	repoPath,
	repoName,
	onSelectWorktree,
	branchPrefix,
}: NotionTaskCardProps) {
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const handleCreateWorktree = useCallback(async () => {
		setCreating(true);
		setError(null);
		try {
			const prefix = branchPrefix || undefined;
			const branchName = task.branch_name
				? generateNotionBranchName(task.branch_name, task.id, prefix)
				: generateNotionBranchName(task.title, task.id, prefix);
			const worktreeDir = computeWorktreeDir(repoPath);
			const worktreePath = `${worktreeDir}/${branchToDir(branchName)}`;

			let defaultBranch: string;
			try {
				defaultBranch = await invoke<string>("get_default_branch", {
					repoPath,
				});
			} catch {
				defaultBranch = "main";
			}

			const entry = await invoke<WorktreeEntry>("create_worktree", {
				repoPath,
				worktreePath,
				branch: branchName,
				createBranch: true,
				baseBranch: defaultBranch,
			});
			onSelectWorktree(entry.path, branchName, repoName);
		} catch (e) {
			setError(String(e));
		} finally {
			setCreating(false);
		}
	}, [
		task.branch_name,
		task.title,
		task.id,
		repoPath,
		repoName,
		onSelectWorktree,
		branchPrefix,
	]);

	const parsedDate = new Date(task.created_at);
	const createdDate = Number.isNaN(parsedDate.getTime())
		? ""
		: parsedDate.toLocaleDateString();

	return (
		<div className="mx-2 mb-1 rounded border border-border bg-card p-2">
			<div className="flex items-start gap-1.5">
				<span className="text-xs font-medium leading-tight flex-1 break-words">
					{task.title}
				</span>
				<a
					href={task.url}
					target="_blank"
					rel="noopener noreferrer"
					className="shrink-0 text-muted-foreground hover:text-foreground"
				>
					<ExternalLink className="size-3" />
				</a>
			</div>

			{Object.keys(task.labels).length > 0 && (
				<div className="flex flex-wrap gap-1 mt-1">
					{Object.entries(task.labels).flatMap(([prop, values]) =>
						values.map((label) => (
							<span
								key={`${prop}:${label}`}
								className="inline-flex items-center rounded-full px-1.5 py-0.5 text-[9px] font-medium leading-none bg-muted text-muted-foreground border border-border"
							>
								{label}
							</span>
						)),
					)}
				</div>
			)}

			<div className="flex items-center gap-2 mt-1.5 text-[10px] text-muted-foreground">
				<span className="ml-auto">{createdDate}</span>
			</div>

			{error && (
				<div className="mt-1.5 text-[10px] text-destructive break-all">
					{error}
				</div>
			)}

			<Button
				variant="outline"
				size="sm"
				className="w-full mt-2 h-6 text-[10px]"
				onClick={handleCreateWorktree}
				disabled={creating}
			>
				{creating ? (
					<Loader2 className="size-3 mr-1 animate-spin" />
				) : (
					<GitBranch className="size-3 mr-1" />
				)}
				Create Worktree
			</Button>
		</div>
	);
}
