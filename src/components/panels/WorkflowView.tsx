import { forwardRef } from "react";
import {
	Group,
	Panel,
	type PanelImperativeHandle,
	type PanelSize,
	Separator,
} from "react-resizable-panels";
import { ImplementationPanel } from "@/components/panels/ImplementationPanel";
import { PlanPanel } from "@/components/panels/PlanPanel";
import {
	TerminalTabPanel,
	type TerminalTabPanelHandle,
} from "@/components/panels/TerminalTabPanel";
import { WorkflowDocumentViewer } from "@/components/panels/WorkflowDocumentViewer";
import type { Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";
import type { TimelineEntry, WorkflowPhase } from "@/types/workflow";

interface WorkflowViewProps {
	rootPath: string;
	theme?: Theme;
	terminalStartupCommand?: string;
	agentType?: string;
	planDocument: string;
	phase: WorkflowPhase;
	planTimeline: TimelineEntry[];
	implTimeline: TimelineEntry[];
	threads: Thread[];
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onRequirementsComplete?: () => void;
	onApprovePlan?: () => void;
	onRequestRevision?: () => void;
	onApprove?: () => void;
	onCreateDocumentThread?: (line: number) => void;
	initialDocTerminalRatio?: [number, number];
	onDocTerminalResize?: (ratios: [number, number]) => void;
	rightPanelRef?: React.Ref<PanelImperativeHandle | null>;
	onRightPanelResize?: (size: PanelSize) => void;
}

export const WorkflowView = forwardRef<
	TerminalTabPanelHandle,
	WorkflowViewProps
>(function WorkflowView(
	{
		rootPath,
		theme,
		terminalStartupCommand,
		agentType,
		planDocument,
		phase,
		planTimeline,
		implTimeline,
		threads,
		onThreadClick,
		onDeleteThread,
		onResolveThread,
		onRequirementsComplete,
		onApprovePlan,
		onRequestRevision,
		onApprove,
		onCreateDocumentThread,
		initialDocTerminalRatio,
		onDocTerminalResize,
		rightPanelRef,
		onRightPanelResize,
	},
	ref,
) {
	const implStarted =
		phase === "implementation" || phase === "review" || phase === "completed";

	return (
		<Group orientation="horizontal" className="h-full">
			{/* Center: Document + Terminal */}
			<Panel id="wf-center" defaultSize="60%" minSize="30%">
				<Group orientation="vertical" className="h-full">
					<Panel
						id="wf-doc"
						defaultSize={
							initialDocTerminalRatio ? `${initialDocTerminalRatio[0]}%` : "60%"
						}
						minSize="20%"
						onResize={(size) =>
							onDocTerminalResize?.([
								size.asPercentage,
								100 - size.asPercentage,
							])
						}
					>
						<WorkflowDocumentViewer
							content={planDocument}
							onCreateThread={onCreateDocumentThread}
						/>
					</Panel>
					<Separator />
					<Panel
						id="wf-terminal"
						defaultSize={
							initialDocTerminalRatio ? `${initialDocTerminalRatio[1]}%` : "40%"
						}
						minSize="15%"
					>
						<div className="h-full border-t border-border">
							<TerminalTabPanel
								ref={ref}
								cwd={rootPath}
								theme={theme}
								terminalStartupCommand={terminalStartupCommand}
								agentType={agentType}
								tabPrefix="MainAgent"
							/>
						</div>
					</Panel>
				</Group>
			</Panel>
			<Separator />
			{/* Right: Plan + Implementation */}
			<Panel
				id="wf-right"
				panelRef={rightPanelRef}
				defaultSize="40%"
				minSize="20%"
				collapsible
				collapsedSize="0%"
				onResize={onRightPanelResize}
			>
				<div className="flex flex-col h-full border-l border-border">
					<Group orientation="vertical" className="h-full">
						<Panel id="wf-plan" defaultSize="50%" minSize="20%">
							<PlanPanel
								timelineEntries={planTimeline}
								threads={threads}
								onThreadClick={onThreadClick}
								onDeleteThread={onDeleteThread}
								onResolveThread={onResolveThread}
								onRequirementsComplete={onRequirementsComplete}
								onRequestRevision={
									phase === "planning" ? onRequestRevision : undefined
								}
							/>
						</Panel>
						<Separator />
						<Panel id="wf-impl" defaultSize="50%" minSize="20%">
							<div className="border-t border-border h-full">
								<ImplementationPanel
									timelineEntries={implTimeline}
									threads={threads}
									started={implStarted}
									onThreadClick={onThreadClick}
									onDeleteThread={onDeleteThread}
									onResolveThread={onResolveThread}
									onApprovePlan={
										phase === "planning" ? onApprovePlan : undefined
									}
									onRequestRevision={
										phase === "implementation" ? onRequestRevision : undefined
									}
									onApprove={phase === "review" ? onApprove : undefined}
								/>
							</div>
						</Panel>
					</Group>
				</div>
			</Panel>
		</Group>
	);
});
