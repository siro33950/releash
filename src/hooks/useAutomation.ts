import { useCallback, useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type { DiagnosticReport, FacetKind } from "@/types/workflow";
import { useStateSubscriptionResult } from "./useStateSubscription";

const EMPTY_REPORT: DiagnosticReport = {
	items: [],
	workflow_summaries: {},
	facet_summaries: {},
	facet_usage: {},
};

export type FacetSubTab = "policy" | "knowledge" | "instruction";

export function useAutomation(open: boolean) {
	const workflowsSubscription = useStateSubscriptionResult(
		open ? "workflows" : null,
	);
	const diagnosticsSubscription = useStateSubscriptionResult(
		open ? "diagnostics" : null,
	);
	const [facetKind, setFacetKind] = useState<FacetKind | null>(null);
	const facetsSubscription = useStateSubscriptionResult(
		open && facetKind ? { kind: "facets", args: [facetKind] } : null,
	);
	const [operationError, setOperationError] = useState<string | null>(null);

	const [selectedWorkflowName, setSelectedWorkflowName] = useState<
		string | null
	>(null);
	const workflows = workflowsSubscription.value ?? [];
	const selectedSourceFormat = selectedWorkflowName
		? workflows.find((workflow) => workflow.name === selectedWorkflowName)
				?.sourceFormat
		: undefined;
	const workflowSubscription = useStateSubscriptionResult(
		open && selectedWorkflowName
			? { kind: "workflow", args: [selectedWorkflowName] }
			: null,
	);
	const sourceSubscription = useStateSubscriptionResult(
		open && selectedWorkflowName && selectedSourceFormat === "yaml"
			? { kind: "workflow-source", args: [selectedWorkflowName] }
			: null,
	);
	const [selectedWorkflow, setSelectedWorkflow] = useState<
		import("@/types/workflow").WorkflowDefinition | null
	>(null);
	const [selectedWorkflowSource, setSelectedWorkflowSource] = useState<
		string | null
	>(null);

	const [selectedFacet, setSelectedFacet] = useState<{
		kind: FacetKind;
		key: string;
	} | null>(null);
	const facetSubscription = useStateSubscriptionResult(
		open && selectedFacet
			? { kind: "facet", args: [selectedFacet.kind, selectedFacet.key] }
			: null,
	);
	const [selectedFacetContent, setSelectedFacetContent] = useState<
		string | null
	>(null);

	const [externalChangeDetected, setExternalChangeDetected] = useState(false);
	const clearExternalChange = useCallback(() => {
		setExternalChangeDetected(false);
	}, []);
	const sourceSeenFor = useRef<string | null>(null);
	const lastSavedSource = useRef<string | null>(null);
	const facetSeenFor = useRef<string | null>(null);
	const lastSavedFacet = useRef<string | null>(null);

	useEffect(() => {
		if (open) return;
		setSelectedWorkflow(null);
		setSelectedWorkflowName(null);
		setSelectedWorkflowSource(null);
		setSelectedFacet(null);
		setSelectedFacetContent(null);
	}, [open]);

	useEffect(() => {
		if (workflowSubscription.value !== undefined)
			setSelectedWorkflow(workflowSubscription.value);
	}, [workflowSubscription.value]);

	useEffect(() => {
		if (!selectedWorkflowName || selectedSourceFormat === undefined) return;
		if (selectedSourceFormat !== "yaml") {
			setSelectedWorkflowSource(null);
			return;
		}
		const source = sourceSubscription.value;
		if (source === undefined) return;
		if (
			sourceSeenFor.current === selectedWorkflowName &&
			source !== lastSavedSource.current
		)
			setExternalChangeDetected(true);
		sourceSeenFor.current = selectedWorkflowName;
		setSelectedWorkflowSource(source);
	}, [selectedWorkflowName, selectedSourceFormat, sourceSubscription.value]);

	useEffect(() => {
		if (!selectedFacet) return;
		const content = facetSubscription.value;
		if (content === undefined) return;
		const id = `${selectedFacet.kind}/${selectedFacet.key}`;
		if (facetSeenFor.current === id && content !== lastSavedFacet.current)
			setExternalChangeDetected(true);
		facetSeenFor.current = id;
		setSelectedFacetContent(content);
	}, [selectedFacet, facetSubscription.value]);

	// --- Workflow operations ---

	const selectWorkflow = useCallback((name: string) => {
		setOperationError(null);
		setExternalChangeDetected(false);
		sourceSeenFor.current = null;
		setSelectedWorkflowName(name);
	}, []);

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
				lastSavedSource.current = source;
				sourceSeenFor.current = workflow.name;
				setSelectedWorkflow(workflow);
				setSelectedWorkflowName(workflow.name);
				setSelectedWorkflowSource(source);
				return { ok: true as const, workflow };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[],
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
			} catch (e) {
				setOperationError(getErrorMessage(e));
			}
		},
		[selectedWorkflow, selectedWorkflowName],
	);

	const duplicateWorkflow = useCallback(
		async (sourceName: string, newName: string) => {
			try {
				await invoke("duplicate_workflow", {
					sourceName,
					newName,
				});
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[],
	);

	const openWorkflowInEditor = useCallback(async (name: string) => {
		try {
			await invoke("open_workflow_in_editor", { name });
		} catch (e) {
			setOperationError(getErrorMessage(e));
		}
	}, []);

	// --- Facet operations ---

	const selectFacet = useCallback((kind: FacetKind, key: string) => {
		setOperationError(null);
		setExternalChangeDetected(false);
		facetSeenFor.current = null;
		setSelectedFacet({ kind, key });
	}, []);

	const clearFacetSelection = useCallback(() => {
		setSelectedFacet(null);
		setSelectedFacetContent(null);
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
				lastSavedFacet.current = content;
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[],
	);

	const deleteFacet = useCallback(
		async (kind: FacetKind, key: string) => {
			try {
				await invoke("delete_facet", { kind, key });
				if (selectedFacet?.key === key && selectedFacet.kind === kind)
					clearFacetSelection();
			} catch (e) {
				setOperationError(getErrorMessage(e));
			}
		},
		[selectedFacet, clearFacetSelection],
	);

	const duplicateFacet = useCallback(
		async (kind: FacetKind, sourceKey: string, newKey: string) => {
			try {
				await invoke("duplicate_facet", {
					kind,
					sourceKey,
					newKey,
				});
				return { ok: true as const };
			} catch (e) {
				return { ok: false as const, error: getErrorMessage(e) };
			}
		},
		[],
	);

	const openFacetInEditor = useCallback(
		async (kind: FacetKind, key: string) => {
			try {
				await invoke("open_facet_in_editor", { kind, key });
			} catch (e) {
				setOperationError(getErrorMessage(e));
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
				setOperationError(getErrorMessage(e));
				return content;
			}
		},
		[],
	);

	const error =
		operationError ??
		workflowsSubscription.error ??
		diagnosticsSubscription.error ??
		facetsSubscription.error ??
		workflowSubscription.error ??
		sourceSubscription.error ??
		facetSubscription.error;

	return {
		workflows,
		facets: facetsSubscription.value ?? [],
		report: diagnosticsSubscription.value ?? EMPTY_REPORT,
		loading:
			open &&
			(workflowsSubscription.value === undefined ||
				diagnosticsSubscription.value === undefined) &&
			!error,
		error,
		setError: setOperationError,

		externalChangeDetected,
		clearExternalChange,

		selectedWorkflow,
		selectedWorkflowName,
		selectedWorkflowSource,
		selectedFacetContent,
		selectedFacetKey: selectedFacet?.key ?? null,
		selectedFacetKind: selectedFacet?.kind ?? null,

		setFacetKind,

		selectWorkflow,
		saveWorkflowSource,
		deleteWorkflow,
		duplicateWorkflow,
		openWorkflowInEditor,

		selectFacet,
		clearFacetSelection,
		saveFacet,
		deleteFacet,
		duplicateFacet,
		openFacetInEditor,
		renderFacetPreview,
	};
}
