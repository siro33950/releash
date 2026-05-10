import { AlertTriangle, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { FacetSubTab, useAutomation } from "@/hooks/useAutomation";
import type { FacetKind, StepMode, Workflow } from "@/types/workflow";
import { FacetDetail } from "./automation/FacetDetail";
import { FacetEditor } from "./automation/FacetEditor";
import { FacetList } from "./automation/FacetList";
import { NameInputDialog } from "./automation/NameInputDialog";
import type { FacetSlot } from "./automation/StepEditor";
import { facetKindToDirName } from "./automation/utils";
import { WorkflowDetail } from "./automation/WorkflowDetail";
import { WorkflowEditor } from "./automation/WorkflowEditor";
import { WorkflowList } from "./automation/WorkflowList";

const FACET_SUB_TABS: { id: FacetSubTab; label: string }[] = [
	{ id: "policy", label: "Policy" },
	{ id: "knowledge", label: "Knowledge" },
	{ id: "instruction", label: "Instruction" },
	{ id: "output_contract", label: "OutputContract" },
];

export function AutomationSection({
	automation,
}: {
	automation: ReturnType<typeof useAutomation>;
}) {
	const {
		workflows,
		facets,
		report,
		loading,
		error,
		externalChangeDetected,
		clearExternalChange,
		selectedWorkflow,
		selectedFacetContent,
		selectedFacetKey,
		selectedFacetKind,
		fetchFacets,
		selectWorkflow,
		saveWorkflow,
		deleteWorkflow,
		duplicateWorkflow,
		openWorkflowInEditor,
		selectFacet,
		saveFacet,
		deleteFacet,
		duplicateFacet,
		openFacetInEditor,
		loadAllFacetKeys,
		renderFacetPreview,
		setSelectedFacetContent,
		setSelectedFacetKey,
		setSelectedFacetKind,
	} = automation;

	const [tab, setTab] = useState<string>("workflows");
	const [facetSubTab, setFacetSubTab] = useState<FacetSubTab>("policy");
	const [editingWorkflow, setEditingWorkflow] = useState(false);
	const [editingFacet, setEditingFacet] = useState(false);

	// Dialogs
	const [createWorkflowOpen, setCreateWorkflowOpen] = useState(false);
	const [createFacetOpen, setCreateFacetOpen] = useState(false);
	const [duplicateDialogOpen, setDuplicateDialogOpen] = useState(false);
	const [duplicateSource, setDuplicateSource] = useState<{
		type: "workflow" | "facet";
		name: string;
		kind?: FacetKind;
	} | null>(null);

	// Facet keys for workflow editor
	const [allFacetKeys, setAllFacetKeys] = useState<Record<FacetSlot, string[]>>(
		{
			policy: [],
			knowledge: [],
			instruction: [],
			output_contract: [],
		},
	);

	// Load facets when switching to facets tab or changing sub-tab
	useEffect(() => {
		if (tab === "facets") {
			fetchFacets(facetSubTab);
		}
	}, [tab, facetSubTab, fetchFacets]);

	const handleEditWorkflow = useCallback(async () => {
		const keys = await loadAllFacetKeys();
		if (keys) {
			setAllFacetKeys(keys);
		}
		setEditingWorkflow(true);
	}, [loadAllFacetKeys]);

	const handleSaveWorkflow = useCallback(
		async (wf: Workflow, originalName?: string) => {
			// 実行中の workflow を編集する場合は警告を表示
			const targetName = originalName ?? wf.name;
			const isRunning = workflows.some(
				(w) => w.name === targetName && w.is_running,
			);
			if (isRunning) {
				const confirmed = window.confirm(
					`ワークフロー '${targetName}' は現在実行中です。保存すると次回以降の実行に影響しますが、実行中のインスタンスには影響しません。保存しますか？`,
				);
				if (!confirmed) {
					return { ok: false as const, error: "cancelled" };
				}
			}
			const result = await saveWorkflow(wf, originalName);
			if (result.ok) {
				setEditingWorkflow(false);
				selectWorkflow(wf.name);
			}
			return result;
		},
		[saveWorkflow, selectWorkflow, workflows],
	);

	const handleEditFacet = useCallback(() => {
		setEditingFacet(true);
	}, []);

	const handleSaveFacet = useCallback(
		async (content: string) => {
			if (!selectedFacetKind || !selectedFacetKey) {
				return { ok: false as const, error: "No facet selected" };
			}
			const result = await saveFacet(
				selectedFacetKind,
				selectedFacetKey,
				content,
			);
			if (result.ok) {
				setEditingFacet(false);
				selectFacet(selectedFacetKind, selectedFacetKey);
			}
			return result;
		},
		[selectedFacetKind, selectedFacetKey, saveFacet, selectFacet],
	);

	const handleCreateWorkflow = useCallback(
		async (name: string) => {
			const newWorkflow: Workflow = {
				name,
				description: "",
				builtin: false,
				steps: [
					{
						name: "step-1",
						mode: "auto" as StepMode,
						inline_prompt: "TODO: describe what this step should do",
						rules: [],
					},
				],
			};
			const result = await saveWorkflow(newWorkflow);
			if (result.ok) {
				selectWorkflow(name);
			}
			return result;
		},
		[saveWorkflow, selectWorkflow],
	);

	const handleCreateFacet = useCallback(
		async (key: string) => {
			const result = await saveFacet(facetSubTab, key, `# ${key}\n\n`, true);
			if (result.ok) {
				selectFacet(facetSubTab, key);
			}
			return result;
		},
		[facetSubTab, saveFacet, selectFacet],
	);

	const handleDuplicate = useCallback(
		async (newName: string) => {
			if (!duplicateSource) {
				return { ok: false as const, error: "No source" };
			}
			if (duplicateSource.type === "workflow") {
				return duplicateWorkflow(duplicateSource.name, newName);
			}
			if (!duplicateSource.kind) {
				return { ok: false as const, error: "No facet kind" };
			}
			return duplicateFacet(
				duplicateSource.kind,
				duplicateSource.name,
				newName,
			);
		},
		[duplicateSource, duplicateWorkflow, duplicateFacet],
	);

	const handleDeleteFacet = useCallback(
		(key: string) => {
			const dirName = facetKindToDirName(facetSubTab);
			const facetId = `${dirName}/${key}`;
			const usage = report.facet_usage[facetId] ?? [];
			const message =
				usage.length > 0
					? `ファセット '${key}' は ${[...new Set(usage.map((u) => u.workflow_name))].join(", ")} で参照されています。削除しますか？`
					: `ファセット '${key}' を削除しますか？`;
			if (!window.confirm(message)) return;
			deleteFacet(facetSubTab, key);
		},
		[facetSubTab, report, deleteFacet],
	);

	const selectedFacetBuiltin = useMemo(() => {
		if (!selectedFacetKey) return false;
		return facets.find((f) => f.key === selectedFacetKey)?.builtin ?? false;
	}, [selectedFacetKey, facets]);

	const isEditing = editingWorkflow || editingFacet;

	const handleReloadExternal = useCallback(() => {
		clearExternalChange();
		if (editingWorkflow && selectedWorkflow) {
			setEditingWorkflow(false);
			selectWorkflow(selectedWorkflow.name);
		}
		if (editingFacet && selectedFacetKind && selectedFacetKey) {
			setEditingFacet(false);
			selectFacet(selectedFacetKind, selectedFacetKey);
		}
	}, [
		clearExternalChange,
		editingWorkflow,
		editingFacet,
		selectedWorkflow,
		selectedFacetKind,
		selectedFacetKey,
		selectWorkflow,
		selectFacet,
	]);

	if (loading) {
		return (
			<div className="flex items-center gap-2 text-sm text-muted-foreground py-8 justify-center">
				<Loader2 className="size-4 animate-spin" />
				Loading...
			</div>
		);
	}

	return (
		<div className="flex flex-col gap-4">
			{error && <p className="text-xs text-destructive">{error}</p>}

			{isEditing && externalChangeDetected && (
				<div className="flex items-center gap-2 rounded-md border border-yellow-500/50 bg-yellow-500/10 px-3 py-2 text-xs text-yellow-700 dark:text-yellow-400">
					<AlertTriangle className="size-4 shrink-0" />
					<span className="flex-1">
						外部でファイルが変更されました。編集中の内容と競合する可能性があります。
					</span>
					<Button
						variant="outline"
						size="sm"
						className="h-6 text-xs"
						onClick={handleReloadExternal}
					>
						リロード
					</Button>
					<Button
						variant="ghost"
						size="sm"
						className="h-6 text-xs"
						onClick={clearExternalChange}
					>
						編集継続
					</Button>
				</div>
			)}

			<Tabs value={tab} onValueChange={setTab}>
				<TabsList variant="line">
					<TabsTrigger value="workflows">Workflows</TabsTrigger>
					<TabsTrigger value="facets">Facets</TabsTrigger>
				</TabsList>

				<TabsContent value="workflows">
					<div className="flex gap-4 mt-4">
						{/* Left: list */}
						<div className="w-64 shrink-0">
							<WorkflowList
								workflows={workflows}
								report={report}
								selectedName={selectedWorkflow?.name ?? null}
								onSelect={(name) => {
									setEditingWorkflow(false);
									selectWorkflow(name);
								}}
								onDelete={(name) => {
									const isRunning = workflows.some(
										(w) => w.name === name && w.is_running,
									);
									const message = isRunning
										? `ワークフロー '${name}' は現在実行中です。削除しますか？`
										: `ワークフロー '${name}' を削除しますか？`;
									if (!window.confirm(message)) return;
									deleteWorkflow(name);
								}}
								onDuplicate={(name) => {
									setDuplicateSource({
										type: "workflow",
										name,
									});
									setDuplicateDialogOpen(true);
								}}
								onOpenInEditor={openWorkflowInEditor}
								onCreate={() => setCreateWorkflowOpen(true)}
							/>
						</div>

						{/* Right: detail / editor */}
						<div className="flex-1 min-w-0">
							{selectedWorkflow ? (
								editingWorkflow ? (
									<WorkflowEditor
										workflow={selectedWorkflow}
										allFacetKeys={allFacetKeys}
										onSave={handleSaveWorkflow}
										onCancel={() => setEditingWorkflow(false)}
									/>
								) : (
									<WorkflowDetail
										workflow={selectedWorkflow}
										report={report}
										onEdit={handleEditWorkflow}
									/>
								)
							) : (
								<p className="text-sm text-muted-foreground py-8 text-center">
									Select a workflow to view details
								</p>
							)}
						</div>
					</div>
				</TabsContent>

				<TabsContent value="facets">
					<div className="mt-2">
						<Tabs
							value={facetSubTab}
							onValueChange={(v) => {
								setFacetSubTab(v as FacetSubTab);
								setSelectedFacetContent(null);
								setSelectedFacetKey(null);
								setSelectedFacetKind(null);
								setEditingFacet(false);
							}}
						>
							<TabsList variant="line">
								{FACET_SUB_TABS.map((st) => (
									<TabsTrigger key={st.id} value={st.id}>
										{st.label}
									</TabsTrigger>
								))}
							</TabsList>
						</Tabs>

						<div className="flex gap-4 mt-4">
							{/* Left: list */}
							<div className="w-64 shrink-0">
								<FacetList
									facets={facets}
									report={report}
									selectedKey={selectedFacetKey}
									onSelect={(key) => {
										setEditingFacet(false);
										selectFacet(facetSubTab, key);
									}}
									onDelete={handleDeleteFacet}
									onDuplicate={(key) => {
										setDuplicateSource({
											type: "facet",
											name: key,
											kind: facetSubTab,
										});
										setDuplicateDialogOpen(true);
									}}
									onOpenInEditor={(key) => openFacetInEditor(facetSubTab, key)}
									onCreate={() => setCreateFacetOpen(true)}
								/>
							</div>

							{/* Right: detail / editor */}
							<div className="flex-1 min-w-0">
								{selectedFacetContent !== null &&
								selectedFacetKey &&
								selectedFacetKind ? (
									editingFacet ? (
										<FacetEditor
											initialContent={selectedFacetContent}
											facetKey={selectedFacetKey}
											onSave={handleSaveFacet}
											onCancel={() => setEditingFacet(false)}
											renderPreview={renderFacetPreview}
										/>
									) : (
										<FacetDetail
											content={selectedFacetContent}
											facetKey={selectedFacetKey}
											kind={selectedFacetKind}
											builtin={selectedFacetBuiltin}
											report={report}
											onEdit={handleEditFacet}
										/>
									)
								) : (
									<p className="text-sm text-muted-foreground py-8 text-center">
										Select a facet to view details
									</p>
								)}
							</div>
						</div>
					</div>
				</TabsContent>
			</Tabs>

			{/* Create Workflow Dialog */}
			<NameInputDialog
				open={createWorkflowOpen}
				onOpenChange={setCreateWorkflowOpen}
				title="Create Workflow"
				description="Enter a name for the new workflow (alphanumeric, hyphens, underscores only)"
				placeholder="my-workflow"
				onSubmit={handleCreateWorkflow}
			/>

			{/* Create Facet Dialog */}
			<NameInputDialog
				open={createFacetOpen}
				onOpenChange={setCreateFacetOpen}
				title="Create Facet"
				description={`Create a new ${facetSubTab} facet (alphanumeric, hyphens, underscores only)`}
				placeholder="my-facet"
				onSubmit={handleCreateFacet}
			/>

			{/* Duplicate Dialog */}
			<NameInputDialog
				open={duplicateDialogOpen}
				onOpenChange={(v) => {
					setDuplicateDialogOpen(v);
					if (!v) setDuplicateSource(null);
				}}
				title={`Duplicate ${duplicateSource?.type === "workflow" ? "Workflow" : "Facet"}`}
				description={`Enter a new name for the duplicate of "${duplicateSource?.name ?? ""}"`}
				placeholder={`${duplicateSource?.name ?? ""}-custom`}
				onSubmit={handleDuplicate}
			/>
		</div>
	);
}
