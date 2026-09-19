import { useCallback, useEffect, useRef, useState } from "react";
import {
	invokeClient as invoke,
	listenClient as listen,
	onClientRefresh,
	watchClient,
} from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type {
	DiagnosticReport,
	FacetKind,
	FacetSummary,
	WorkflowDefinition,
	WorkflowDefinitionSummary,
} from "@/types/workflow";

const EMPTY_REPORT: DiagnosticReport = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};

export type FacetSubTab = "policy" | "knowledge" | "instruction";

export function useAutomation(open: boolean) {
	const [workflows, setWorkflows] = useState<WorkflowDefinitionSummary[]>([]);
	const facetKind = useRef<FacetKind | null>(null);
	const [facets, setFacets] = useState<FacetSummary[]>([]);
	const [report, setReport] = useState<DiagnosticReport>(EMPTY_REPORT);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const [selectedWorkflow, setSelectedWorkflow] =
		useState<WorkflowDefinition | null>(null);
	const [selectedWorkflowName, setSelectedWorkflowName] = useState<
		string | null
	>(null);
	const [selectedWorkflowSource, setSelectedWorkflowSource] = useState<
		string | null
	>(null);
	const [selectedFacetContent, setSelectedFacetContent] = useState<
		string | null
	>(null);
	const [selectedFacetKey, setSelectedFacetKey] = useState<string | null>(null);
	const [selectedFacetKind, setSelectedFacetKind] = useState<FacetKind | null>(
		null,
	);

	const [externalChangeDetected, setExternalChangeDetected] = useState(false);

	const clearExternalChange = useCallback(() => {
		setExternalChangeDetected(false);
	}, []);

	const fetchAll = useCallback(async () => {
		setLoading(true);
		setError(null);
		try {
			const [wfList, diagReport] = await Promise.all([
				invoke("list_workflows"),
				invoke("diagnose_all_cmd"),
			]);
			setWorkflows(wfList);
			setReport(diagReport);
		} catch (e) {
			setError(getErrorMessage(e));
		} finally {
			setLoading(false);
		}
	}, []);

	const fetchFacets = useCallback(async (kind: FacetKind) => {
		facetKind.current = kind;
		try {
			const list = await invoke("list_facet_summaries", {
				kind,
			});
			if (facetKind.current === kind) setFacets(list);
		} catch (e) {
			setError(getErrorMessage(e));
		}
	}, []);

	const refreshDiagnostics = useCallback(async () => {
		try {
			const diagReport = await invoke("diagnose_all_cmd");
			setReport(diagReport);
		} catch (e) {
			setError(getErrorMessage(e));
		}
	}, []);

	useEffect(() => {
		if (open) {
			fetchAll();
		}
		return () => {
			setSelectedWorkflow(null);
			setSelectedWorkflowName(null);
			setSelectedWorkflowSource(null);
			setSelectedFacetContent(null);
			setSelectedFacetKey(null);
			setSelectedFacetKind(null);
		};
	}, [open, fetchAll]);

	// File watcher for workflow/facet directory changes
	useEffect(() => {
		if (!open) return;

		let disposed = false;
		let unlisten: (() => void) | null = null;
		let watcherId: number | null = null;
		let stopWatch: (() => void) | undefined;

		let preparation = 0;
		const prepareWatcher = async () => {
			if (stopWatch) return;
			const current = ++preparation;
			try {
				const dir = await invoke("get_automation_config_dir");
				if (disposed || current !== preparation) return;
				stopWatch = watchClient("start_watching", { path: dir }, (id) => {
					watcherId = id;
				});
			} catch (e) {
				if (!disposed && current === preparation) setError(getErrorMessage(e));
			}
		};
		const refresh = () => {
			void fetchAll();
			if (facetKind.current) void fetchFacets(facetKind.current);
		};
		const setup = async () => {
			const off = await listen(
				"file-change",
				(event) => {
					if (
						!disposed &&
						watcherId !== null &&
						event.payload.watcher_id === watcherId
					) {
						setExternalChangeDetected(true);
						refresh();
					}
				},
				() => {
					refresh();
					void prepareWatcher();
				},
			);
			if (disposed) {
				off();
				return;
			}
			unlisten = off;
			await prepareWatcher();
		};
		void setup();

		return () => {
			disposed = true;
			unlisten?.();
			stopWatch?.();
		};
	}, [open, fetchAll, fetchFacets]);

	// --- Workflow operations ---

	const selectionSequence = useRef(0);
	const selectWorkflow = useCallback(
		async (name: string) => {
			const sequence = ++selectionSequence.current;
			setSelectedWorkflowName(name);
			setError(null);
			try {
				const sourceFormat =
					workflows.find((workflow) => workflow.name === name)?.sourceFormat ??
					"yaml";
				if (sourceFormat === "yaml") {
					const source = await invoke("get_workflow_source", { name });
					if (sequence !== selectionSequence.current) return;
					setSelectedWorkflowSource(source);
				} else {
					setSelectedWorkflowSource(null);
				}
				try {
					const wf = await invoke("get_workflow", { name });
					if (sequence !== selectionSequence.current) return;
					setSelectedWorkflow(wf);
				} catch (e) {
					if (sequence !== selectionSequence.current) return;
					setSelectedWorkflow(null);
					setError(getErrorMessage(e));
					await refreshDiagnostics();
				}
			} catch (e) {
				if (sequence !== selectionSequence.current) return;
				setSelectedWorkflow(null);
				setSelectedWorkflowSource(null);
				setError(getErrorMessage(e));
			}
		},
		[refreshDiagnostics, workflows],
	);

	useEffect(() => {
		if (!open || !selectedWorkflowName) return;
		return onClientRefresh(() => {
			void selectWorkflow(selectedWorkflowName);
		});
	}, [open, selectedWorkflowName, selectWorkflow]);

	const saveWorkflowSource = useCallback(
		async (source: string, originalName?: string) => {
			try {
				const response = await invoke("save_workflow_source", {
					source,
					originalName: originalName ?? null,
				});
				if (!response.ok) {
					return {
						ok: false as const,
						error: response.error ?? "workflow_diagnostics",
						diagnostics: response.diagnostics,
					};
				}
				const { workflow } = response;
				setSelectedWorkflow(workflow);
				setSelectedWorkflowName(workflow.name);
				setSelectedWorkflowSource(source);
				await fetchAll();
				return { ok: true as const, workflow };
			} catch (e) {
				await refreshDiagnostics();
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[fetchAll, refreshDiagnostics],
	);

	const deleteWorkflow = useCallback(
		async (name: string) => {
			try {
				await invoke("delete_workflow", { name });
				if ((selectedWorkflow?.name ?? selectedWorkflowName) === name) {
					setSelectedWorkflow(null);
					setSelectedWorkflowName(null);
					setSelectedWorkflowSource(null);
				}
				await fetchAll();
			} catch (e) {
				setError(getErrorMessage(e));
			}
		},
		[fetchAll, selectedWorkflow, selectedWorkflowName],
	);

	const duplicateWorkflow = useCallback(
		async (sourceName: string, newName: string) => {
			try {
				await invoke("duplicate_workflow", {
					sourceName,
					newName,
				});
				await fetchAll();
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[fetchAll],
	);

	const openWorkflowInEditor = useCallback(async (name: string) => {
		try {
			await invoke("open_workflow_in_editor", { name });
		} catch (e) {
			setError(getErrorMessage(e));
		}
	}, []);

	// --- Facet operations ---

	const selectFacet = useCallback(async (kind: FacetKind, key: string) => {
		try {
			const content = await invoke("get_facet", { kind, key });
			setSelectedFacetContent(content);
			setSelectedFacetKey(key);
			setSelectedFacetKind(kind);
		} catch (e) {
			setError(getErrorMessage(e));
		}
	}, []);

	const saveFacet = useCallback(
		async (kind: FacetKind, key: string, content: string, isNew?: boolean) => {
			try {
				await invoke("save_facet", {
					kind,
					key,
					content,
					isNew: isNew ?? null,
				});
				await Promise.all([fetchFacets(kind), refreshDiagnostics()]);
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[fetchFacets, refreshDiagnostics],
	);

	const deleteFacet = useCallback(
		async (kind: FacetKind, key: string) => {
			try {
				await invoke("delete_facet", { kind, key });
				if (selectedFacetKey === key && selectedFacetKind === kind) {
					setSelectedFacetContent(null);
					setSelectedFacetKey(null);
					setSelectedFacetKind(null);
				}
				await Promise.all([fetchFacets(kind), refreshDiagnostics()]);
			} catch (e) {
				setError(getErrorMessage(e));
			}
		},
		[fetchFacets, refreshDiagnostics, selectedFacetKey, selectedFacetKind],
	);

	const duplicateFacet = useCallback(
		async (kind: FacetKind, sourceKey: string, newKey: string) => {
			try {
				await invoke("duplicate_facet", {
					kind,
					sourceKey,
					newKey,
				});
				await Promise.all([fetchFacets(kind), refreshDiagnostics()]);
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[fetchFacets, refreshDiagnostics],
	);

	const openFacetInEditor = useCallback(
		async (kind: FacetKind, key: string) => {
			try {
				await invoke("open_facet_in_editor", { kind, key });
			} catch (e) {
				setError(getErrorMessage(e));
			}
		},
		[],
	);

	const renderFacetPreview = useCallback(
		async (content: string, sampleValues: Record<string, string>) => {
			try {
				const rendered = await invoke("render_facet_preview", {
					content,
					sampleValues,
				});
				return rendered;
			} catch (e) {
				setError(getErrorMessage(e));
				return content;
			}
		},
		[],
	);

	return {
		workflows,
		facets,
		report,
		loading,
		error,
		setError,

		externalChangeDetected,
		clearExternalChange,

		selectedWorkflow,
		selectedWorkflowName,
		selectedWorkflowSource,
		selectedFacetContent,
		selectedFacetKey,
		selectedFacetKind,

		fetchAll,
		fetchFacets,
		refreshDiagnostics,

		selectWorkflow,
		saveWorkflowSource,
		deleteWorkflow,
		duplicateWorkflow,
		openWorkflowInEditor,

		selectFacet,
		saveFacet,
		deleteFacet,
		duplicateFacet,
		openFacetInEditor,
		renderFacetPreview,

		setSelectedWorkflow,
		setSelectedWorkflowSource,
		setSelectedFacetContent,
		setSelectedFacetKey,
		setSelectedFacetKind,
	};
}
